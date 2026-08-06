use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::io::{Read, Write};

use crate::Config;
use crate::cache::RecentCache;
use crate::context::ContextIndex;
use crate::dictionary::{Dictionary, Stats, SurfaceRecord, TemplateRecord};
use crate::feature::Feature;
use crate::item::{Item, SurfaceId, TemplateId};
use crate::matcher::{CandidateMatcher, PartialIndex};
use crate::normalizer::{MAX_SLOTS_PER_ITEM, Normalizer, bound_slots};
use crate::ppm::{ContextState, FollowerState, Ppm};
use crate::predictor::Predictor;
use crate::stream::{StreamId, StreamState, StreamTable};
use crate::tokenizer::{TokenIndex, Tokenizer};

const MAGIC: &[u8; 8] = b"VISTA\0\r\n";
const VERSION: u32 = 1;
const FEATURE_FLAGS: u64 =
    (cfg!(feature = "surface-indexes") as u64) | ((cfg!(feature = "recent-cache") as u64) << 1);
const CONFIG_WORDS: usize = 20;
const MAX_STRING_BYTES: usize = 16 * 1024 * 1024;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug)]
pub enum SnapshotError {
    Io(std::io::Error),
    InvalidMagic,
    UnsupportedVersion(u32),
    UnsupportedFeatures(u64),
    IncompatibleConfig,
    Corrupt(&'static str),
    LimitExceeded(&'static str),
    ChecksumMismatch,
    TrailingData,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "snapshot I/O failed: {error}"),
            Self::InvalidMagic => formatter.write_str("invalid Vista snapshot magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported Vista snapshot version {version}")
            }
            Self::UnsupportedFeatures(features) => {
                write!(
                    formatter,
                    "unsupported Vista snapshot features {features:#x}"
                )
            }
            Self::IncompatibleConfig => formatter.write_str("snapshot configuration mismatch"),
            Self::Corrupt(section) => write!(formatter, "corrupt Vista snapshot {section}"),
            Self::LimitExceeded(section) => {
                write!(formatter, "Vista snapshot exceeds configured {section}")
            }
            Self::ChecksumMismatch => formatter.write_str("Vista snapshot checksum mismatch"),
            Self::TrailingData => formatter.write_str("Vista snapshot contains trailing data"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<std::io::Error> for SnapshotError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl Predictor {
    pub fn write_snapshot<W: Write>(&self, writer: W) -> Result<(), SnapshotError> {
        let mut output = DigestWriter::new(writer);
        output.bytes(MAGIC)?;
        output.u32(VERSION)?;
        output.u64(FEATURE_FLAGS)?;
        output.u64(config_fingerprint(&self.config))?;
        for word in config_words(&self.config) {
            output.u64(word)?;
        }
        output.string(self.normalizer.snapshot_key())?;
        output.string(self.tokenizer.snapshot_key())?;
        output.string(self.matcher.snapshot_key())?;
        output.u64(self.clock)?;
        output.u32(self.dictionary.next_template)?;
        output.u32(self.dictionary.next_surface)?;

        output.len(self.dictionary.templates.len())?;
        for (id, record) in &self.dictionary.templates {
            output.u32(id.0)?;
            write_item(&mut output, &record.item)?;
            write_stats(&mut output, &record.stats)?;
        }
        output.len(self.dictionary.surfaces.len())?;
        for (id, record) in &self.dictionary.surfaces {
            output.u32(id.0)?;
            output.u32(record.template.0)?;
            write_item(&mut output, &record.item)?;
            write_stats(&mut output, &record.stats)?;
            output.len(record.slots.len())?;
            for feature in &record.slots {
                write_feature(&mut output, feature)?;
            }
        }

        output.u64(self.ppm.zero_total)?;
        output.len(self.ppm.zero.len())?;
        for (id, count) in &self.ppm.zero {
            output.u32(id.0)?;
            output.u64(*count)?;
        }
        output.len(self.ppm.contexts.len())?;
        for (context, state) in &self.ppm.contexts {
            output.len(context.len())?;
            for id in context {
                output.u32(id.0)?;
            }
            output.u64(state.total)?;
            output.u64(state.pruned_count)?;
            output.u64(state.last_seen)?;
            output.len(state.followers.len())?;
            for (id, follower) in &state.followers {
                output.u32(id.0)?;
                output.u64(follower.count)?;
                output.u64(follower.last_seen)?;
            }
        }

        output.len(self.streams.streams.len())?;
        for (id, state) in &self.streams.streams {
            output.u64(id.0)?;
            output.option_u64(state.last_position)?;
            output.u64(state.last_seen)?;
            output.len(state.recent.len())?;
            for template in &state.recent {
                output.u32(template.0)?;
            }
        }
        write_cache(&mut output, &self.cache)?;
        write_nested_counts(&mut output, &self.context.items, |output, id| {
            output.u32(id.0)
        })?;
        write_nested_counts(&mut output, &self.tokens.items, |output, id| {
            output.u32(id.0)
        })?;
        write_nested_counts(&mut output, &self.partials.items, |output, id| {
            output.u32(id.0)
        })?;
        let (mut writer, checksum) = output.finish();
        writer.write_all(&checksum.to_le_bytes())?;
        Ok(())
    }

    pub fn read_snapshot<R, N, T, M>(
        config: Config,
        normalizer: N,
        tokenizer: T,
        matcher: M,
        reader: R,
    ) -> Result<Self, SnapshotError>
    where
        R: Read,
        N: Normalizer + 'static,
        T: Tokenizer + 'static,
        M: CandidateMatcher + 'static,
    {
        let config = config.normalise();
        let mut input = DigestReader::new(reader);
        let mut magic = [0_u8; 8];
        input.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(SnapshotError::InvalidMagic);
        }
        let version = input.u32()?;
        if version != VERSION {
            return Err(SnapshotError::UnsupportedVersion(version));
        }
        let feature_flags = input.u64()?;
        if feature_flags != FEATURE_FLAGS {
            return Err(SnapshotError::UnsupportedFeatures(feature_flags));
        }
        if input.u64()? != config_fingerprint(&config) {
            return Err(SnapshotError::IncompatibleConfig);
        }
        for expected in config_words(&config) {
            if input.u64()? != expected {
                return Err(SnapshotError::IncompatibleConfig);
            }
        }
        if input.string()? != normalizer.snapshot_key()
            || input.string()? != tokenizer.snapshot_key()
            || input.string()? != matcher.snapshot_key()
        {
            return Err(SnapshotError::IncompatibleConfig);
        }
        let clock = input.u64()?;
        let next_template = input.u32()?;
        let next_surface = input.u32()?;

        let template_count = input.count(config.max_templates, "templates")?;
        let mut templates = BTreeMap::new();
        for _ in 0..template_count {
            let id = TemplateId(input.u32()?);
            let record = TemplateRecord {
                item: read_item(&mut input)?,
                surfaces: BTreeSet::new(),
                stats: read_stats(&mut input, clock)?,
            };
            if templates.insert(id, record).is_some() {
                return Err(SnapshotError::Corrupt("template IDs"));
            }
        }
        let surface_count = input.count(config.max_surfaces, "surfaces")?;
        let mut surfaces = BTreeMap::new();
        for _ in 0..surface_count {
            let id = SurfaceId(input.u32()?);
            let template = TemplateId(input.u32()?);
            if !templates.contains_key(&template) {
                return Err(SnapshotError::Corrupt("surface template"));
            }
            let item = read_item(&mut input)?;
            let stats = read_stats(&mut input, clock)?;
            let slot_count = input.count(MAX_SLOTS_PER_ITEM, "surface slots")?;
            let mut slots = Vec::new();
            for _ in 0..slot_count {
                slots.push(read_feature(&mut input)?);
            }
            let record = SurfaceRecord {
                item,
                template,
                slots,
                stats,
            };
            if surfaces.insert(id, record).is_some() {
                return Err(SnapshotError::Corrupt("surface IDs"));
            }
            templates
                .get_mut(&template)
                .ok_or(SnapshotError::Corrupt("surface template"))?
                .surfaces
                .insert(id);
        }
        for record in templates.values() {
            if record.surfaces.is_empty() {
                return Err(SnapshotError::Corrupt("orphan template"));
            }
            let mut retained_surface_count = 0_u64;
            for surface in &record.surfaces {
                let surface = surfaces
                    .get(surface)
                    .ok_or(SnapshotError::Corrupt("template surface"))?;
                retained_surface_count = retained_surface_count
                    .checked_add(surface.stats.count)
                    .ok_or(SnapshotError::Corrupt("surface count overflow"))?;
                if surface.stats.last_seen > record.stats.last_seen {
                    return Err(SnapshotError::Corrupt("surface recency"));
                }
            }
            if retained_surface_count > record.stats.count {
                return Err(SnapshotError::Corrupt("template count"));
            }
        }
        let dictionary = Dictionary::restore(
            config.max_templates,
            config.max_surfaces,
            templates,
            surfaces,
            next_template,
            next_surface,
        )
        .ok_or(SnapshotError::Corrupt("dictionary"))?;
        let zero_total = input.u64()?;
        let zero_count = input.count(config.max_templates, "zero-order counts")?;
        let mut zero = BTreeMap::new();
        for _ in 0..zero_count {
            let id = TemplateId(input.u32()?);
            let count = input.u64()?;
            if count == 0
                || !dictionary.templates.contains_key(&id)
                || zero.insert(id, count).is_some()
            {
                return Err(SnapshotError::Corrupt("zero-order counts"));
            }
        }
        if checked_sum(zero.values().copied())? != zero_total {
            return Err(SnapshotError::Corrupt("zero-order total"));
        }
        if zero.len() != dictionary.templates.len()
            || dictionary
                .templates
                .iter()
                .any(|(id, record)| zero.get(id).copied() != Some(record.stats.count))
        {
            return Err(SnapshotError::Corrupt("zero-order dictionary"));
        }
        let context_count = input.count(config.max_contexts, "contexts")?;
        let mut contexts = BTreeMap::new();
        for _ in 0..context_count {
            let depth = input.count(config.max_order, "context depth")?;
            if depth == 0 {
                return Err(SnapshotError::Corrupt("empty context"));
            }
            let mut context = Vec::new();
            for _ in 0..depth {
                let id = TemplateId(input.u32()?);
                if !dictionary.templates.contains_key(&id) {
                    return Err(SnapshotError::Corrupt("context template"));
                }
                context.push(id);
            }
            let total = input.u64()?;
            let pruned_count = input.u64()?;
            let last_seen = input.u64()?;
            if last_seen == 0 || last_seen > clock || total.checked_add(pruned_count).is_none() {
                return Err(SnapshotError::Corrupt("context statistics"));
            }
            let follower_count =
                input.count(config.max_followers_per_context, "context followers")?;
            if follower_count == 0 || total == 0 {
                return Err(SnapshotError::Corrupt("context followers"));
            }
            let mut followers = BTreeMap::new();
            for _ in 0..follower_count {
                let id = TemplateId(input.u32()?);
                let count = input.u64()?;
                let follower_last_seen = input.u64()?;
                if count == 0
                    || follower_last_seen == 0
                    || follower_last_seen > clock
                    || follower_last_seen > last_seen
                    || !dictionary.templates.contains_key(&id)
                    || followers
                        .insert(
                            id,
                            FollowerState {
                                count,
                                last_seen: follower_last_seen,
                            },
                        )
                        .is_some()
                {
                    return Err(SnapshotError::Corrupt("context follower"));
                }
            }
            if checked_sum(followers.values().map(|follower| follower.count))? != total {
                return Err(SnapshotError::Corrupt("context total"));
            }
            if contexts
                .insert(
                    context,
                    ContextState {
                        followers,
                        total,
                        pruned_count,
                        last_seen,
                    },
                )
                .is_some()
            {
                return Err(SnapshotError::Corrupt("duplicate context"));
            }
        }
        let ppm = Ppm::restore(
            contexts,
            zero,
            zero_total,
            config.max_contexts,
            config.max_followers_per_context,
            config.max_order,
        );

        let stream_count = input.count(config.max_streams, "streams")?;
        let mut stream_map = BTreeMap::new();
        for _ in 0..stream_count {
            let id = StreamId(input.u64()?);
            let last_position = input.option_u64()?;
            let last_seen = input.u64()?;
            if last_seen > clock {
                return Err(SnapshotError::Corrupt("stream clock"));
            }
            let recent_count = input.count(config.max_order, "stream history")?;
            let mut recent = VecDeque::new();
            for _ in 0..recent_count {
                let template = TemplateId(input.u32()?);
                if !dictionary.templates.contains_key(&template) {
                    return Err(SnapshotError::Corrupt("stream template"));
                }
                recent.push_back(template);
            }
            if recent.is_empty() != last_position.is_none()
                || (!recent.is_empty() && last_seen == 0)
            {
                return Err(SnapshotError::Corrupt("stream continuity"));
            }
            if stream_map
                .insert(
                    id,
                    StreamState {
                        last_position,
                        recent,
                        last_seen,
                    },
                )
                .is_some()
            {
                return Err(SnapshotError::Corrupt("duplicate stream"));
            }
        }
        let streams = StreamTable {
            streams: stream_map,
            capacity: config.max_streams,
        };
        let cache = read_cache(&mut input, &config, &dictionary, clock)?;
        if cache
            .streams
            .keys()
            .any(|stream| !streams.streams.contains_key(stream))
        {
            return Err(SnapshotError::Corrupt("stream cache"));
        }
        let context_items = read_nested_counts(
            &mut input,
            config.max_context_associations,
            config.max_context_associations,
            config.max_surfaces,
            "context associations",
            |input| Ok(SurfaceId(input.u32()?)),
        )?;
        let max_token_associations = config
            .max_tokens
            .checked_mul(config.max_surface_candidates_per_template)
            .ok_or(SnapshotError::LimitExceeded("token associations"))?;
        let token_items = read_nested_counts(
            &mut input,
            config.max_tokens,
            max_token_associations,
            config.max_surfaces,
            "token associations",
            |input| Ok(SurfaceId(input.u32()?)),
        )?;
        let partial_items = read_nested_counts(
            &mut input,
            config.max_partial_associations,
            config.max_partial_associations,
            config.max_surfaces,
            "partial associations",
            |input| Ok(SurfaceId(input.u32()?)),
        )?;
        for id in context_items
            .values()
            .chain(token_items.values())
            .chain(partial_items.values())
            .flat_map(BTreeMap::keys)
        {
            if !dictionary.surfaces.contains_key(id) {
                return Err(SnapshotError::Corrupt("surface association"));
            }
        }
        for items in [&context_items, &token_items, &partial_items] {
            for (surface, count) in items.values().flat_map(BTreeMap::iter) {
                if dictionary
                    .surface(*surface)
                    .is_none_or(|record| *count > record.stats.count)
                {
                    return Err(SnapshotError::Corrupt("surface association count"));
                }
            }
        }
        let (reader, checksum) = input.finish();
        verify_checksum_and_eof(reader, checksum)?;
        for surface in dictionary.surfaces.values() {
            let normalized = bound_slots(normalizer.normalize(&surface.item));
            let template = dictionary
                .template(surface.template)
                .ok_or(SnapshotError::Corrupt("surface template"))?;
            if normalized.template != template.item
                || !features_equal(&normalized.slots, &surface.slots)
            {
                return Err(SnapshotError::IncompatibleConfig);
            }
        }

        let mut predictor =
            Predictor::with_components(config.clone(), normalizer, tokenizer, matcher);
        predictor.dictionary = dictionary;
        predictor.streams = streams;
        predictor.ppm = ppm;
        predictor.cache = cache;
        predictor.context = ContextIndex::restore(context_items, config.max_context_associations);
        predictor.tokens = TokenIndex::restore(
            token_items,
            config.max_tokens,
            config.max_surface_candidates_per_template,
        );
        predictor.partials = PartialIndex::restore(
            partial_items,
            config.max_partial_associations,
            config.max_partial_chars_per_item,
        );
        predictor.clock = clock;
        Ok(predictor)
    }
}

fn config_fingerprint(config: &Config) -> u64 {
    let mut hash = FNV_OFFSET;
    for value in config_words(config) {
        digest(&mut hash, &value.to_le_bytes());
    }
    hash
}

fn config_words(config: &Config) -> [u64; CONFIG_WORDS] {
    [
        config.max_templates as u64,
        config.max_surfaces as u64,
        config.max_streams as u64,
        config.max_order as u64,
        config.max_contexts as u64,
        config.max_followers_per_context as u64,
        config.max_context_associations as u64,
        config.max_tokens as u64,
        config.max_partial_chars_per_item as u64,
        config.max_partial_associations as u64,
        config.max_candidate_templates as u64,
        config.max_surface_candidates_per_template as u64,
        config.max_candidates as u64,
        config.recent_cache_items as u64,
        config.recent_cache_weight.to_bits(),
        config.recent_cache_half_life,
        config.weights.context.to_bits(),
        config.weights.surface.to_bits(),
        config.weights.outcome.to_bits(),
        config.weights.partial.to_bits(),
    ]
}

fn write_item<W: Write>(output: &mut DigestWriter<W>, item: &Item) -> Result<(), SnapshotError> {
    output.string(&item.namespace)?;
    output.string(&item.value)
}

fn read_item<R: Read>(input: &mut DigestReader<R>) -> Result<Item, SnapshotError> {
    Ok(Item::new(input.string()?, input.string()?))
}

fn write_stats<W: Write>(output: &mut DigestWriter<W>, stats: &Stats) -> Result<(), SnapshotError> {
    output.u64(stats.count)?;
    output.u64(stats.last_seen)?;
    output.u64(stats.outcome_sum.to_bits())?;
    output.u64(stats.outcome_count)
}

fn read_stats<R: Read>(input: &mut DigestReader<R>, clock: u64) -> Result<Stats, SnapshotError> {
    let stats = Stats {
        count: input.u64()?,
        last_seen: input.u64()?,
        outcome_sum: f64::from_bits(input.u64()?),
        outcome_count: input.u64()?,
    };
    if stats.count == 0
        || stats.last_seen == 0
        || stats.last_seen > clock
        || !stats.outcome_sum.is_finite()
        || stats.outcome_sum < 0.0
        || stats.outcome_sum > stats.outcome_count as f64
        || (stats.outcome_count == 0 && stats.outcome_sum != 0.0)
    {
        return Err(SnapshotError::Corrupt("outcome stats"));
    }
    Ok(stats)
}

fn write_feature<W: Write>(
    output: &mut DigestWriter<W>,
    feature: &Feature,
) -> Result<(), SnapshotError> {
    match feature {
        Feature::Categorical { name, value } => {
            output.u8(0)?;
            output.string(name)?;
            output.string(value)
        }
        Feature::Numeric { name, value } => {
            output.u8(1)?;
            output.string(name)?;
            output.u32(value.to_bits())
        }
    }
}

fn read_feature<R: Read>(input: &mut DigestReader<R>) -> Result<Feature, SnapshotError> {
    match input.u8()? {
        0 => Ok(Feature::categorical(input.string()?, input.string()?)),
        1 => {
            let name = input.string()?;
            let value = f32::from_bits(input.u32()?);
            Ok(Feature::numeric(name, value))
        }
        _ => Err(SnapshotError::Corrupt("feature tag")),
    }
}

fn features_equal(left: &[Feature], right: &[Feature]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (
                    Feature::Categorical {
                        name: left_name,
                        value: left_value,
                    },
                    Feature::Categorical {
                        name: right_name,
                        value: right_value,
                    },
                ) => left_name == right_name && left_value == right_value,
                (
                    Feature::Numeric {
                        name: left_name,
                        value: left_value,
                    },
                    Feature::Numeric {
                        name: right_name,
                        value: right_value,
                    },
                ) => left_name == right_name && left_value.to_bits() == right_value.to_bits(),
                _ => false,
            })
}

fn write_cache<W: Write>(
    output: &mut DigestWriter<W>,
    cache: &RecentCache,
) -> Result<(), SnapshotError> {
    output.len(cache.global.len())?;
    for (clock, previous, id) in &cache.global {
        output.u64(*clock)?;
        write_optional_template(output, *previous)?;
        output.u32(id.0)?;
    }
    output.len(cache.streams.len())?;
    for (stream, entries) in &cache.streams {
        output.u64(stream.0)?;
        output.len(entries.len())?;
        for (clock, previous, id) in entries {
            output.u64(*clock)?;
            write_optional_template(output, *previous)?;
            output.u32(id.0)?;
        }
    }
    Ok(())
}

fn read_cache<R: Read>(
    input: &mut DigestReader<R>,
    config: &Config,
    dictionary: &Dictionary,
    model_clock: u64,
) -> Result<RecentCache, SnapshotError> {
    let global_count = input.count(config.recent_cache_items, "global cache")?;
    let mut global = VecDeque::<(u64, Option<TemplateId>, TemplateId)>::new();
    for _ in 0..global_count {
        let entry = (
            input.u64()?,
            read_optional_template(input)?,
            TemplateId(input.u32()?),
        );
        if entry.0 == 0
            || entry.0 > model_clock
            || global.back().is_some_and(|previous| previous.0 > entry.0)
            || entry
                .1
                .is_some_and(|id| !dictionary.templates.contains_key(&id))
            || !dictionary.templates.contains_key(&entry.2)
        {
            return Err(SnapshotError::Corrupt("global cache template"));
        }
        global.push_back(entry);
    }
    let stream_count = input.count(config.max_streams, "stream caches")?;
    let mut streams = BTreeMap::new();
    for _ in 0..stream_count {
        let stream = StreamId(input.u64()?);
        let count = input.count(config.recent_cache_items, "stream cache")?;
        if count == 0 {
            return Err(SnapshotError::Corrupt("empty stream cache"));
        }
        let mut entries = VecDeque::<(u64, Option<TemplateId>, TemplateId)>::new();
        for _ in 0..count {
            let entry = (
                input.u64()?,
                read_optional_template(input)?,
                TemplateId(input.u32()?),
            );
            if entry.0 == 0
                || entry.0 > model_clock
                || entries.back().is_some_and(|previous| previous.0 > entry.0)
                || entry
                    .1
                    .is_some_and(|id| !dictionary.templates.contains_key(&id))
                || !dictionary.templates.contains_key(&entry.2)
            {
                return Err(SnapshotError::Corrupt("stream cache template"));
            }
            entries.push_back(entry);
        }
        if streams.insert(stream, entries).is_some() {
            return Err(SnapshotError::Corrupt("duplicate stream cache"));
        }
    }
    Ok(RecentCache {
        global,
        streams,
        capacity: config.recent_cache_items,
        half_life: config.recent_cache_half_life,
        max_streams: config.max_streams,
    })
}

fn write_optional_template<W: Write>(
    output: &mut DigestWriter<W>,
    template: Option<TemplateId>,
) -> Result<(), SnapshotError> {
    match template {
        Some(template) => {
            output.u8(1)?;
            output.u32(template.0)
        }
        None => output.u8(0),
    }
}

fn read_optional_template<R: Read>(
    input: &mut DigestReader<R>,
) -> Result<Option<TemplateId>, SnapshotError> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(TemplateId(input.u32()?))),
        _ => Err(SnapshotError::Corrupt("optional template")),
    }
}

fn write_nested_counts<W, K, F>(
    output: &mut DigestWriter<W>,
    values: &BTreeMap<String, BTreeMap<K, u64>>,
    mut write_key: F,
) -> Result<(), SnapshotError>
where
    W: Write,
    K: Ord,
    F: FnMut(&mut DigestWriter<W>, &K) -> Result<(), SnapshotError>,
{
    output.len(values.len())?;
    for (key, counts) in values {
        output.string(key)?;
        output.len(counts.len())?;
        for (id, count) in counts {
            write_key(output, id)?;
            output.u64(*count)?;
        }
    }
    Ok(())
}

fn read_nested_counts<R, K, F>(
    input: &mut DigestReader<R>,
    max_keys: usize,
    max_associations: usize,
    max_per_key: usize,
    section: &'static str,
    mut read_key: F,
) -> Result<BTreeMap<String, BTreeMap<K, u64>>, SnapshotError>
where
    R: Read,
    K: Ord,
    F: FnMut(&mut DigestReader<R>) -> Result<K, SnapshotError>,
{
    let outer = input.count(max_keys.max(1), section)?;
    let mut result = BTreeMap::new();
    let mut associations = 0usize;
    for _ in 0..outer {
        let key = input.string()?;
        if key.is_empty() {
            return Err(SnapshotError::Corrupt(section));
        }
        let count = input.count(max_per_key, section)?;
        if count == 0 {
            return Err(SnapshotError::Corrupt(section));
        }
        associations = associations
            .checked_add(count)
            .ok_or(SnapshotError::LimitExceeded(section))?;
        if associations > max_associations {
            return Err(SnapshotError::LimitExceeded(section));
        }
        let mut values = BTreeMap::new();
        for _ in 0..count {
            let id = read_key(input)?;
            let count = input.u64()?;
            if count == 0 || values.insert(id, count).is_some() {
                return Err(SnapshotError::Corrupt(section));
            }
        }
        if result.insert(key, values).is_some() {
            return Err(SnapshotError::Corrupt(section));
        }
    }
    Ok(result)
}

struct DigestWriter<W> {
    writer: W,
    hash: u64,
}

impl<W: Write> DigestWriter<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            hash: FNV_OFFSET,
        }
    }
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.writer.write_all(bytes)?;
        digest(&mut self.hash, bytes);
        Ok(())
    }
    fn u8(&mut self, value: u8) -> Result<(), SnapshotError> {
        self.bytes(&[value])
    }
    fn u32(&mut self, value: u32) -> Result<(), SnapshotError> {
        self.bytes(&value.to_le_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<(), SnapshotError> {
        self.bytes(&value.to_le_bytes())
    }
    fn len(&mut self, value: usize) -> Result<(), SnapshotError> {
        self.u64(u64::try_from(value).map_err(|_| SnapshotError::LimitExceeded("length"))?)
    }
    fn string(&mut self, value: &str) -> Result<(), SnapshotError> {
        if value.len() > MAX_STRING_BYTES {
            return Err(SnapshotError::LimitExceeded("string"));
        }
        self.len(value.len())?;
        self.bytes(value.as_bytes())
    }
    fn option_u64(&mut self, value: Option<u64>) -> Result<(), SnapshotError> {
        match value {
            Some(value) => {
                self.u8(1)?;
                self.u64(value)
            }
            None => self.u8(0),
        }
    }
    fn finish(self) -> (W, u64) {
        (self.writer, self.hash)
    }
}

struct DigestReader<R> {
    reader: R,
    hash: u64,
}

impl<R: Read> DigestReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            hash: FNV_OFFSET,
        }
    }
    fn read_exact(&mut self, bytes: &mut [u8]) -> Result<(), SnapshotError> {
        self.reader.read_exact(bytes)?;
        digest(&mut self.hash, bytes);
        Ok(())
    }
    fn u8(&mut self) -> Result<u8, SnapshotError> {
        let mut value = [0; 1];
        self.read_exact(&mut value)?;
        Ok(value[0])
    }
    fn u32(&mut self) -> Result<u32, SnapshotError> {
        let mut value = [0; 4];
        self.read_exact(&mut value)?;
        Ok(u32::from_le_bytes(value))
    }
    fn u64(&mut self) -> Result<u64, SnapshotError> {
        let mut value = [0; 8];
        self.read_exact(&mut value)?;
        Ok(u64::from_le_bytes(value))
    }
    fn count(&mut self, max: usize, section: &'static str) -> Result<usize, SnapshotError> {
        let value =
            usize::try_from(self.u64()?).map_err(|_| SnapshotError::LimitExceeded(section))?;
        if value > max {
            return Err(SnapshotError::LimitExceeded(section));
        }
        Ok(value)
    }
    fn string(&mut self) -> Result<String, SnapshotError> {
        let length = self.count(MAX_STRING_BYTES, "string")?;
        let mut bytes = vec![0; length];
        self.read_exact(&mut bytes)?;
        String::from_utf8(bytes).map_err(|_| SnapshotError::Corrupt("UTF-8 string"))
    }
    fn option_u64(&mut self) -> Result<Option<u64>, SnapshotError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            _ => Err(SnapshotError::Corrupt("option tag")),
        }
    }
    fn finish(self) -> (R, u64) {
        (self.reader, self.hash)
    }
}

fn verify_checksum_and_eof<R: Read>(mut reader: R, checksum: u64) -> Result<(), SnapshotError> {
    let mut expected = [0_u8; 8];
    reader.read_exact(&mut expected)?;
    if u64::from_le_bytes(expected) != checksum {
        return Err(SnapshotError::ChecksumMismatch);
    }
    let mut trailing = [0_u8; 1];
    if reader.read(&mut trailing)? != 0 {
        return Err(SnapshotError::TrailingData);
    }
    Ok(())
}

fn digest(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn checked_sum<I>(values: I) -> Result<u64, SnapshotError>
where
    I: IntoIterator<Item = u64>,
{
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(SnapshotError::Corrupt("count overflow"))
    })
}
