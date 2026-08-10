use super::*;

impl Predictor {
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
        let mut input =
            DigestReader::new(reader, config.max_snapshot_bytes, config.max_string_bytes);
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
        if input.metadata_string()? != normalizer.snapshot_key()
            || input.metadata_string()? != tokenizer.snapshot_key()
            || input.metadata_string()? != matcher.snapshot_key()
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
        let correction_count = input.count(config.max_correction_pairs, "correction pairs")?;
        let mut corrections = Vec::with_capacity(correction_count);
        for _ in 0..correction_count {
            let typed = read_item(&mut input)?;
            let corrected = read_item(&mut input)?;
            let count = input.u64()?;
            if count == 0 {
                return Err(SnapshotError::Corrupt("correction count"));
            }
            corrections.push((CorrectionPair { typed, corrected }, count));
        }
        let (reader, checksum, bytes_read, max_bytes) = input.finish();
        verify_checksum_and_eof(reader, checksum, bytes_read, max_bytes)?;
        for surface in dictionary.surfaces.values() {
            let normalized = normalizer.normalize(&surface.item);
            if normalized.slots.len() > MAX_SLOTS_PER_ITEM {
                return Err(SnapshotError::IncompatibleConfig);
            }
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
        predictor.corrections = CorrectionLog::restore(config.max_correction_pairs, corrections);
        predictor.clock = clock;
        if predictor.retained_string_bytes() > config.max_retained_string_bytes {
            return Err(SnapshotError::LimitExceeded("retained string bytes"));
        }
        Ok(predictor)
    }
}
