use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};

use crate::api::{Config, Feature, Item, StreamId, TemplateId};
use crate::model::{Dictionary, RecentCache, Stats};

use super::{CONFIG_WORDS, FNV_OFFSET, FNV_PRIME, SnapshotError};

pub(super) fn config_fingerprint(config: &Config) -> u64 {
    let mut hash = FNV_OFFSET;
    for value in config_words(config) {
        digest(&mut hash, &value.to_le_bytes());
    }
    hash
}

pub(super) fn config_words(config: &Config) -> [u64; CONFIG_WORDS] {
    [
        config.max_string_bytes as u64,
        config.max_retained_string_bytes as u64,
        config.max_snapshot_bytes as u64,
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
        config.max_repair_iterations as u64,
        config.max_correction_pairs as u64,
        config.channel_weight.to_bits(),
        config.weights.context.to_bits(),
        config.weights.surface.to_bits(),
        config.weights.outcome.to_bits(),
        config.weights.partial.to_bits(),
    ]
}

pub(super) fn write_item<W: Write>(
    output: &mut DigestWriter<W>,
    item: &Item,
) -> Result<(), SnapshotError> {
    output.string(&item.namespace)?;
    output.string(&item.value)
}

pub(super) fn read_item<R: Read>(input: &mut DigestReader<R>) -> Result<Item, SnapshotError> {
    Ok(Item::new(input.string()?, input.string()?))
}

pub(super) fn write_stats<W: Write>(
    output: &mut DigestWriter<W>,
    stats: &Stats,
) -> Result<(), SnapshotError> {
    output.u64(stats.count)?;
    output.u64(stats.last_seen)?;
    output.u64(stats.outcome_sum.to_bits())?;
    output.u64(stats.outcome_count)
}

pub(super) fn read_stats<R: Read>(
    input: &mut DigestReader<R>,
    clock: u64,
) -> Result<Stats, SnapshotError> {
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

pub(super) fn write_feature<W: Write>(
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

pub(super) fn read_feature<R: Read>(input: &mut DigestReader<R>) -> Result<Feature, SnapshotError> {
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

pub(super) fn write_cache<W: Write>(
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

pub(super) fn read_cache<R: Read>(
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

pub(super) fn write_optional_template<W: Write>(
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

pub(super) fn read_optional_template<R: Read>(
    input: &mut DigestReader<R>,
) -> Result<Option<TemplateId>, SnapshotError> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(TemplateId(input.u32()?))),
        _ => Err(SnapshotError::Corrupt("optional template")),
    }
}

pub(super) fn write_nested_counts<W, K, F>(
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

pub(super) fn read_nested_counts<R, K, F>(
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

pub(super) struct DigestWriter<W> {
    writer: W,
    hash: u64,
    bytes_written: usize,
    max_bytes: usize,
    max_string_bytes: usize,
}

impl<W: Write> DigestWriter<W> {
    pub(super) fn new(writer: W, max_bytes: usize, max_string_bytes: usize) -> Self {
        Self {
            writer,
            hash: FNV_OFFSET,
            bytes_written: 0,
            max_bytes,
            max_string_bytes,
        }
    }
    pub(super) fn bytes(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let total = self
            .bytes_written
            .checked_add(bytes.len())
            .and_then(|value| value.checked_add(8))
            .ok_or(SnapshotError::LimitExceeded("total bytes"))?;
        if total > self.max_bytes {
            return Err(SnapshotError::LimitExceeded("total bytes"));
        }
        self.writer.write_all(bytes)?;
        digest(&mut self.hash, bytes);
        self.bytes_written += bytes.len();
        Ok(())
    }
    pub(super) fn u8(&mut self, value: u8) -> Result<(), SnapshotError> {
        self.bytes(&[value])
    }
    pub(super) fn u32(&mut self, value: u32) -> Result<(), SnapshotError> {
        self.bytes(&value.to_le_bytes())
    }
    pub(super) fn u64(&mut self, value: u64) -> Result<(), SnapshotError> {
        self.bytes(&value.to_le_bytes())
    }
    pub(super) fn len(&mut self, value: usize) -> Result<(), SnapshotError> {
        self.u64(u64::try_from(value).map_err(|_| SnapshotError::LimitExceeded("length"))?)
    }
    pub(super) fn string(&mut self, value: &str) -> Result<(), SnapshotError> {
        if value.len() > self.max_string_bytes {
            return Err(SnapshotError::LimitExceeded("string"));
        }
        self.len(value.len())?;
        self.bytes(value.as_bytes())
    }
    pub(super) fn metadata_string(&mut self, value: &str) -> Result<(), SnapshotError> {
        self.len(value.len())?;
        self.bytes(value.as_bytes())
    }
    pub(super) fn option_u64(&mut self, value: Option<u64>) -> Result<(), SnapshotError> {
        match value {
            Some(value) => {
                self.u8(1)?;
                self.u64(value)
            }
            None => self.u8(0),
        }
    }
    pub(super) fn finish(mut self) -> Result<W, SnapshotError> {
        self.writer.write_all(&self.hash.to_le_bytes())?;
        Ok(self.writer)
    }
}

pub(super) struct DigestReader<R> {
    reader: R,
    hash: u64,
    bytes_read: usize,
    max_bytes: usize,
    max_string_bytes: usize,
}

impl<R: Read> DigestReader<R> {
    pub(super) fn new(reader: R, max_bytes: usize, max_string_bytes: usize) -> Self {
        Self {
            reader,
            hash: FNV_OFFSET,
            bytes_read: 0,
            max_bytes,
            max_string_bytes,
        }
    }
    pub(super) fn read_exact(&mut self, bytes: &mut [u8]) -> Result<(), SnapshotError> {
        let total = self
            .bytes_read
            .checked_add(bytes.len())
            .and_then(|value| value.checked_add(8))
            .ok_or(SnapshotError::LimitExceeded("total bytes"))?;
        if total > self.max_bytes {
            return Err(SnapshotError::LimitExceeded("total bytes"));
        }
        self.reader.read_exact(bytes)?;
        digest(&mut self.hash, bytes);
        self.bytes_read += bytes.len();
        Ok(())
    }
    pub(super) fn u8(&mut self) -> Result<u8, SnapshotError> {
        let mut value = [0; 1];
        self.read_exact(&mut value)?;
        Ok(value[0])
    }
    pub(super) fn u32(&mut self) -> Result<u32, SnapshotError> {
        let mut value = [0; 4];
        self.read_exact(&mut value)?;
        Ok(u32::from_le_bytes(value))
    }
    pub(super) fn u64(&mut self) -> Result<u64, SnapshotError> {
        let mut value = [0; 8];
        self.read_exact(&mut value)?;
        Ok(u64::from_le_bytes(value))
    }
    pub(super) fn count(
        &mut self,
        max: usize,
        section: &'static str,
    ) -> Result<usize, SnapshotError> {
        let value =
            usize::try_from(self.u64()?).map_err(|_| SnapshotError::LimitExceeded(section))?;
        if value > max {
            return Err(SnapshotError::LimitExceeded(section));
        }
        Ok(value)
    }
    pub(super) fn string(&mut self) -> Result<String, SnapshotError> {
        let length = self.count(self.max_string_bytes, "string")?;
        let mut bytes = vec![0; length];
        self.read_exact(&mut bytes)?;
        String::from_utf8(bytes).map_err(|_| SnapshotError::Corrupt("UTF-8 string"))
    }
    pub(super) fn metadata_string(&mut self) -> Result<String, SnapshotError> {
        let length = self.count(self.max_bytes, "metadata string")?;
        let mut bytes = vec![0; length];
        self.read_exact(&mut bytes)?;
        String::from_utf8(bytes).map_err(|_| SnapshotError::Corrupt("UTF-8 string"))
    }
    pub(super) fn option_u64(&mut self) -> Result<Option<u64>, SnapshotError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            _ => Err(SnapshotError::Corrupt("option tag")),
        }
    }
    pub(super) fn finish(self) -> (R, u64, usize, usize) {
        (self.reader, self.hash, self.bytes_read, self.max_bytes)
    }
}

pub(super) fn verify_checksum_and_eof<R: Read>(
    mut reader: R,
    checksum: u64,
    bytes_read: usize,
    max_bytes: usize,
) -> Result<(), SnapshotError> {
    if bytes_read
        .checked_add(8)
        .is_none_or(|total| total > max_bytes)
    {
        return Err(SnapshotError::LimitExceeded("total bytes"));
    }
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

pub(super) fn digest(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}
