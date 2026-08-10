use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;

use crate::{IdentityNormalizer, Item, Normalizer, Observation, StreamId};

/// Deterministic integer sequences for external classical-model comparisons.
pub struct ResearchExport {
    pub dictionary: Vec<Item>,
    pub sequences: Vec<Vec<u32>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResearchExportError {
    VocabularyExhausted,
}

impl fmt::Display for ResearchExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("research export vocabulary exceeds u32 identifiers")
    }
}

impl std::error::Error for ResearchExportError {}

impl ResearchExport {
    pub fn from_observations<I>(observations: I) -> Result<Self, ResearchExportError>
    where
        I: IntoIterator<Item = Observation>,
    {
        Self::with_normalizer(observations, IdentityNormalizer)
    }

    pub fn with_normalizer<I, N>(
        observations: I,
        normalizer: N,
    ) -> Result<Self, ResearchExportError>
    where
        I: IntoIterator<Item = Observation>,
        N: Normalizer,
    {
        let mut ids = BTreeMap::<Item, u32>::new();
        let mut dictionary = Vec::new();
        let mut live = BTreeMap::<StreamId, (u64, usize, Vec<u32>)>::new();
        let mut ordered_sequences = Vec::new();
        for (ordinal, observation) in observations.into_iter().enumerate() {
            let template = normalizer.normalize(&observation.item).template;
            let id = if let Some(id) = ids.get(&template) {
                *id
            } else {
                let id = dictionary
                    .len()
                    .checked_add(1)
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or(ResearchExportError::VocabularyExhausted)?;
                ids.insert(template.clone(), id);
                dictionary.push(template);
                id
            };
            let stream = live
                .entry(observation.stream)
                .or_insert_with(|| (0, ordinal, Vec::new()));
            if !stream.2.is_empty() && stream.0.checked_add(1) != Some(observation.position.0) {
                ordered_sequences.push((stream.1, std::mem::take(&mut stream.2)));
                stream.1 = ordinal;
            }
            stream.0 = observation.position.0;
            stream.2.push(id);
        }
        ordered_sequences.extend(live.into_values().filter_map(|(_, start, sequence)| {
            (!sequence.is_empty()).then_some((start, sequence))
        }));
        ordered_sequences.sort_by_key(|(start, _)| *start);
        let sequences = ordered_sequences
            .into_iter()
            .map(|(_, sequence)| sequence)
            .collect();
        Ok(Self {
            dictionary,
            sequences,
        })
    }

    pub fn write_spmf<W: Write>(&self, mut writer: W) -> std::io::Result<()> {
        for sequence in &self.sequences {
            for id in sequence {
                write!(writer, "{id} -1 ")?;
            }
            writeln!(writer, "-2")?;
        }
        Ok(())
    }

    pub fn write_plain<W: Write>(&self, mut writer: W) -> std::io::Result<()> {
        for sequence in &self.sequences {
            for (index, id) in sequence.iter().enumerate() {
                if index > 0 {
                    writer.write_all(b" ")?;
                }
                write!(writer, "{id}")?;
            }
            writer.write_all(b"\n")?;
        }
        Ok(())
    }

    pub fn write_dictionary<W: Write>(&self, mut writer: W) -> std::io::Result<()> {
        for (index, item) in self.dictionary.iter().enumerate() {
            let id = index + 1;
            writeln!(
                writer,
                "{id}\t{}\t{}",
                escape_field(&item.namespace),
                escape_field(&item.value)
            )?;
        }
        Ok(())
    }
}

fn escape_field(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            '\n' => escaped.push_str("\\n"),
            character => escaped.push(character),
        }
    }
    escaped
}
