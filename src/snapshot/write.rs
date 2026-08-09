use super::*;

impl Predictor {
    pub fn write_snapshot<W: Write>(&self, writer: W) -> Result<(), SnapshotError> {
        if self.retained_string_bytes() > self.config.max_retained_string_bytes {
            return Err(SnapshotError::LimitExceeded("retained string bytes"));
        }
        let mut output = DigestWriter::new(
            writer,
            self.config.max_snapshot_bytes,
            self.config.max_string_bytes,
        );
        output.bytes(MAGIC)?;
        output.u32(VERSION)?;
        output.u64(FEATURE_FLAGS)?;
        output.u64(config_fingerprint(&self.config))?;
        for word in config_words(&self.config) {
            output.u64(word)?;
        }
        output.metadata_string(self.normalizer.snapshot_key())?;
        output.metadata_string(self.tokenizer.snapshot_key())?;
        output.metadata_string(self.matcher.snapshot_key())?;
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
        let contexts = self.ppm.ordered_contexts();
        output.len(contexts.len())?;
        for (context, state) in contexts {
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
        let corrections = self.corrections.ordered();
        output.len(corrections.len())?;
        for (pair, count) in corrections {
            write_item(&mut output, &pair.typed)?;
            write_item(&mut output, &pair.corrected)?;
            output.u64(count)?;
        }
        let _ = output.finish()?;
        Ok(())
    }
}
