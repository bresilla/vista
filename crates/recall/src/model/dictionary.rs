use std::collections::{BTreeMap, BTreeSet};

use crate::adapters::NormalizedItem;
use crate::api::{Feature, InputError, Item, SurfaceId, TemplateId};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Stats {
    pub(crate) count: u64,
    pub(crate) last_seen: u64,
    pub(crate) outcome_sum: f64,
    pub(crate) outcome_count: u64,
}

impl Stats {
    pub(crate) fn quality(&self) -> Option<f64> {
        (self.outcome_count > 0).then(|| self.outcome_sum / self.outcome_count as f64)
    }

    fn record(&mut self, outcome: &[Feature], clock: u64) {
        self.count = self.count.saturating_add(1);
        self.last_seen = clock;
        for quality in outcome.iter().filter_map(Feature::quality) {
            self.outcome_sum = (self.outcome_sum + f64::from(quality)).min(u64::MAX as f64);
            self.outcome_count = self.outcome_count.saturating_add(1);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TemplateRecord {
    pub(crate) item: Item,
    pub(crate) surfaces: BTreeSet<SurfaceId>,
    pub(crate) stats: Stats,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SurfaceRecord {
    pub(crate) item: Item,
    pub(crate) template: TemplateId,
    pub(crate) slots: Vec<Feature>,
    pub(crate) stats: Stats,
}

pub(crate) struct Admission {
    pub(crate) template: TemplateId,
    #[cfg(feature = "surface-indexes")]
    pub(crate) surface: SurfaceId,
    pub(crate) removed_templates: Vec<TemplateId>,
    pub(crate) removed_surfaces: Vec<SurfaceId>,
}

#[derive(Clone)]
pub(crate) struct Dictionary {
    pub(crate) templates: BTreeMap<TemplateId, TemplateRecord>,
    pub(crate) surfaces: BTreeMap<SurfaceId, SurfaceRecord>,
    template_ids: BTreeMap<u64, Vec<TemplateId>>,
    surface_ids: BTreeMap<u64, Vec<SurfaceId>>,
    template_order: BTreeSet<(u64, u64, TemplateId)>,
    surface_order: BTreeSet<(u64, u64, SurfaceId)>,
    template_popularity: BTreeSet<(u64, u64, TemplateId)>,
    surface_popularity: BTreeMap<TemplateId, BTreeSet<(u64, u64, SurfaceId)>>,
    pub(crate) next_template: u32,
    pub(crate) next_surface: u32,
    max_templates: usize,
    max_surfaces: usize,
    string_bytes: usize,
}

impl Dictionary {
    pub(crate) fn new(max_templates: usize, max_surfaces: usize) -> Self {
        Self {
            templates: BTreeMap::new(),
            surfaces: BTreeMap::new(),
            template_ids: BTreeMap::new(),
            surface_ids: BTreeMap::new(),
            template_order: BTreeSet::new(),
            surface_order: BTreeSet::new(),
            template_popularity: BTreeSet::new(),
            surface_popularity: BTreeMap::new(),
            next_template: 0,
            next_surface: 0,
            max_templates,
            max_surfaces,
            string_bytes: 0,
        }
    }

    pub(crate) fn admit(
        &mut self,
        raw: &Item,
        normalized: NormalizedItem,
        outcome: &[Feature],
        clock: u64,
    ) -> Option<Admission> {
        let mut removed_templates = Vec::new();
        let mut removed_surfaces = Vec::new();
        let existing_template = self.template_id(&normalized.template);
        let existing_surface = self.surface_id(raw);
        if let Some(surface) = existing_surface {
            let retained_template = self.surfaces.get(&surface)?.template;
            if existing_template != Some(retained_template) {
                return None;
            }
        }
        if existing_template.is_none() && self.next_template.checked_add(1).is_none()
            || existing_surface.is_none() && self.next_surface.checked_add(1).is_none()
        {
            return None;
        }

        let template = if let Some(id) = existing_template {
            id
        } else {
            let id = TemplateId(self.next_template);
            let next_template = self.next_template + 1;
            if self.templates.len() >= self.max_templates {
                let victim = self.template_order.first().map(|entry| entry.2)?;
                let (_, surfaces) = self.remove_template(victim)?;
                removed_templates.push(victim);
                removed_surfaces.extend(surfaces);
            }
            self.next_template = next_template;
            let record = TemplateRecord {
                item: normalized.template.clone(),
                surfaces: BTreeSet::new(),
                stats: Stats::default(),
            };
            self.template_ids
                .entry(item_hash(&normalized.template))
                .or_default()
                .push(id);
            self.template_order.insert((0, 0, id));
            self.template_popularity.insert((0, 0, id));
            self.templates.insert(id, record);
            self.string_bytes = self
                .string_bytes
                .saturating_add(item_string_bytes(&normalized.template));
            id
        };

        let surface = if let Some(id) = existing_surface {
            id
        } else {
            let id = SurfaceId(self.next_surface);
            let next_surface = self.next_surface + 1;
            if self.surfaces.len() >= self.max_surfaces {
                let victim = self.surface_order.first().map(|entry| entry.2)?;
                if let Some(evicted) = self.remove_surface(victim) {
                    removed_surfaces.push(victim);
                    let empty_template = self
                        .templates
                        .get(&evicted.template)
                        .is_some_and(|record| record.surfaces.is_empty());
                    if empty_template && evicted.template != template {
                        self.remove_template(evicted.template);
                        removed_templates.push(evicted.template);
                    }
                }
            }
            self.next_surface = next_surface;
            let retained_bytes =
                item_string_bytes(raw).saturating_add(feature_string_bytes(&normalized.slots));
            let record = SurfaceRecord {
                item: raw.clone(),
                template,
                slots: normalized.slots,
                stats: Stats::default(),
            };
            self.surface_ids.entry(item_hash(raw)).or_default().push(id);
            self.surface_order.insert((0, 0, id));
            self.surface_popularity
                .entry(template)
                .or_default()
                .insert((0, 0, id));
            self.surfaces.insert(id, record);
            self.string_bytes = self.string_bytes.saturating_add(retained_bytes);
            self.templates.get_mut(&template)?.surfaces.insert(id);
            id
        };

        if self.surfaces.get(&surface)?.template != template {
            return None;
        }
        self.touch_template(template, outcome, clock);
        self.touch_surface(surface, outcome, clock);
        Some(Admission {
            template,
            #[cfg(feature = "surface-indexes")]
            surface,
            removed_templates,
            removed_surfaces,
        })
    }

    pub(crate) fn validate_admission(
        &self,
        raw: &Item,
        normalized: &NormalizedItem,
    ) -> Result<(), InputError> {
        let template = self.template_id(&normalized.template);
        let surface = self.surface_id(raw);
        if let Some(surface) = surface {
            let retained = self
                .surfaces
                .get(&surface)
                .ok_or(InputError::InconsistentNormalization)?;
            if template != Some(retained.template) || retained.slots != normalized.slots {
                return Err(InputError::InconsistentNormalization);
            }
        }
        if template.is_none() && self.next_template.checked_add(1).is_none() {
            return Err(InputError::IdentifierExhausted("template"));
        }
        if surface.is_none() && self.next_surface.checked_add(1).is_none() {
            return Err(InputError::IdentifierExhausted("surface"));
        }
        Ok(())
    }

    pub(crate) fn additional_string_bytes(&self, raw: &Item, normalized: &NormalizedItem) -> usize {
        let template = if self.template_id(&normalized.template).is_none() {
            item_string_bytes(&normalized.template)
        } else {
            0
        };
        let surface = if self.surface_id(raw).is_none() {
            item_string_bytes(raw).saturating_add(feature_string_bytes(&normalized.slots))
        } else {
            0
        };
        template.saturating_add(surface)
    }

    pub(crate) fn string_bytes(&self) -> usize {
        self.string_bytes
    }

    fn touch_template(&mut self, id: TemplateId, outcome: &[Feature], clock: u64) {
        if let Some(record) = self.templates.get_mut(&id) {
            self.template_order
                .remove(&(record.stats.last_seen, record.stats.count, id));
            self.template_popularity
                .remove(&(record.stats.count, record.stats.last_seen, id));
            record.stats.record(outcome, clock);
            self.template_order
                .insert((record.stats.last_seen, record.stats.count, id));
            self.template_popularity
                .insert((record.stats.count, record.stats.last_seen, id));
        }
    }

    fn touch_surface(&mut self, id: SurfaceId, outcome: &[Feature], clock: u64) {
        if let Some(record) = self.surfaces.get_mut(&id) {
            self.surface_order
                .remove(&(record.stats.last_seen, record.stats.count, id));
            if let Some(ranked) = self.surface_popularity.get_mut(&record.template) {
                ranked.remove(&(record.stats.count, record.stats.last_seen, id));
            }
            record.stats.record(outcome, clock);
            self.surface_order
                .insert((record.stats.last_seen, record.stats.count, id));
            self.surface_popularity
                .entry(record.template)
                .or_default()
                .insert((record.stats.count, record.stats.last_seen, id));
        }
    }

    pub(crate) fn remove_surface(&mut self, id: SurfaceId) -> Option<SurfaceRecord> {
        let record = self.surfaces.remove(&id)?;
        self.string_bytes = self
            .string_bytes
            .saturating_sub(item_string_bytes(&record.item))
            .saturating_sub(feature_string_bytes(&record.slots));
        remove_reverse_id(&mut self.surface_ids, item_hash(&record.item), id);
        self.surface_order
            .remove(&(record.stats.last_seen, record.stats.count, id));
        if let Some(ranked) = self.surface_popularity.get_mut(&record.template) {
            ranked.remove(&(record.stats.count, record.stats.last_seen, id));
            if ranked.is_empty() {
                self.surface_popularity.remove(&record.template);
            }
        }
        if let Some(template) = self.templates.get_mut(&record.template) {
            template.surfaces.remove(&id);
        }
        Some(record)
    }

    pub(crate) fn remove_template(
        &mut self,
        id: TemplateId,
    ) -> Option<(TemplateRecord, Vec<SurfaceId>)> {
        let record = self.templates.remove(&id)?;
        self.string_bytes = self
            .string_bytes
            .saturating_sub(item_string_bytes(&record.item));
        remove_reverse_id(&mut self.template_ids, item_hash(&record.item), id);
        self.template_order
            .remove(&(record.stats.last_seen, record.stats.count, id));
        self.template_popularity
            .remove(&(record.stats.count, record.stats.last_seen, id));
        self.surface_popularity.remove(&id);
        let surfaces: Vec<_> = record.surfaces.iter().copied().collect();
        for surface in &surfaces {
            self.remove_surface(*surface);
        }
        Some((record, surfaces))
    }

    pub(crate) fn template_id(&self, item: &Item) -> Option<TemplateId> {
        self.template_ids.get(&item_hash(item)).and_then(|ids| {
            ids.iter().copied().find(|id| {
                self.templates
                    .get(id)
                    .is_some_and(|record| record.item == *item)
            })
        })
    }

    pub(crate) fn surface_id(&self, item: &Item) -> Option<SurfaceId> {
        self.surface_ids.get(&item_hash(item)).and_then(|ids| {
            ids.iter().copied().find(|id| {
                self.surfaces
                    .get(id)
                    .is_some_and(|record| record.item == *item)
            })
        })
    }

    pub(crate) fn template(&self, id: TemplateId) -> Option<&TemplateRecord> {
        self.templates.get(&id)
    }

    pub(crate) fn surface(&self, id: SurfaceId) -> Option<&SurfaceRecord> {
        self.surfaces.get(&id)
    }

    pub(crate) fn global_templates(&self, limit: usize) -> Vec<TemplateId> {
        self.template_popularity
            .iter()
            .rev()
            .take(limit)
            .map(|(_, _, id)| *id)
            .collect()
    }

    pub(crate) fn surfaces_for(&self, template: TemplateId, limit: usize) -> Vec<SurfaceId> {
        let Some(ranked) = self.surface_popularity.get(&template) else {
            return Vec::new();
        };
        ranked
            .iter()
            .rev()
            .take(limit)
            .map(|(_, _, id)| *id)
            .collect()
    }

    pub(crate) fn clear(&mut self) {
        self.templates.clear();
        self.surfaces.clear();
        self.template_ids.clear();
        self.surface_ids.clear();
        self.template_order.clear();
        self.surface_order.clear();
        self.template_popularity.clear();
        self.surface_popularity.clear();
        self.next_template = 0;
        self.next_surface = 0;
        self.string_bytes = 0;
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn restore(
        max_templates: usize,
        max_surfaces: usize,
        templates: BTreeMap<TemplateId, TemplateRecord>,
        surfaces: BTreeMap<SurfaceId, SurfaceRecord>,
        next_template: u32,
        next_surface: u32,
    ) -> Option<Self> {
        if templates
            .keys()
            .next_back()
            .is_some_and(|id| id.0 >= next_template)
            || surfaces
                .keys()
                .next_back()
                .is_some_and(|id| id.0 >= next_surface)
        {
            return None;
        }
        let mut dictionary = Self::new(max_templates, max_surfaces);
        dictionary.next_template = next_template;
        dictionary.next_surface = next_surface;
        for (id, record) in templates {
            if dictionary.template_id(&record.item).is_some() {
                return None;
            }
            dictionary
                .template_ids
                .entry(item_hash(&record.item))
                .or_default()
                .push(id);
            dictionary
                .template_order
                .insert((record.stats.last_seen, record.stats.count, id));
            dictionary
                .template_popularity
                .insert((record.stats.count, record.stats.last_seen, id));
            dictionary.templates.insert(id, record);
        }
        for (id, record) in surfaces {
            if !dictionary.templates.contains_key(&record.template)
                || dictionary.surface_id(&record.item).is_some()
            {
                return None;
            }
            dictionary
                .surface_ids
                .entry(item_hash(&record.item))
                .or_default()
                .push(id);
            dictionary
                .surface_order
                .insert((record.stats.last_seen, record.stats.count, id));
            dictionary
                .surface_popularity
                .entry(record.template)
                .or_default()
                .insert((record.stats.count, record.stats.last_seen, id));
            dictionary.surfaces.insert(id, record);
        }
        dictionary.string_bytes = dictionary
            .templates
            .values()
            .map(|record| item_string_bytes(&record.item))
            .chain(dictionary.surfaces.values().map(|record| {
                item_string_bytes(&record.item).saturating_add(feature_string_bytes(&record.slots))
            }))
            .fold(0_usize, usize::saturating_add);
        Some(dictionary)
    }
}

fn item_string_bytes(item: &Item) -> usize {
    item.namespace.len().saturating_add(item.value.len())
}

fn feature_string_bytes(features: &[Feature]) -> usize {
    features
        .iter()
        .map(|feature| match feature {
            Feature::Categorical { name, value } => name.len().saturating_add(value.len()),
            Feature::Numeric { name, .. } => name.len(),
        })
        .fold(0_usize, usize::saturating_add)
}

fn item_hash(item: &Item) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for bytes in [item.namespace.as_bytes(), item.value.as_bytes()] {
        for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn remove_reverse_id<I: Copy + Eq>(index: &mut BTreeMap<u64, Vec<I>>, hash: u64, id: I) {
    if let Some(ids) = index.get_mut(&hash) {
        ids.retain(|candidate| *candidate != id);
        if ids.is_empty() {
            index.remove(&hash);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_exhaustion_does_not_evict_retained_state() {
        let mut dictionary = Dictionary::new(1, 1);
        let first = Item::new("test", "first");
        dictionary
            .admit(
                &first,
                NormalizedItem {
                    template: first.clone(),
                    slots: Vec::new(),
                },
                &[],
                1,
            )
            .unwrap();
        dictionary.next_template = u32::MAX;
        let second = Item::new("test", "second");
        assert!(
            dictionary
                .admit(
                    &second,
                    NormalizedItem {
                        template: second.clone(),
                        slots: Vec::new(),
                    },
                    &[],
                    2,
                )
                .is_none()
        );
        assert_eq!(dictionary.templates.len(), 1);
        assert_eq!(dictionary.surfaces.len(), 1);
        assert_eq!(dictionary.surfaces.values().next().unwrap().item, first);
    }

    #[test]
    fn surface_identifier_exhaustion_is_transactional() {
        let mut dictionary = Dictionary::new(2, 2);
        let first = Item::new("test", "first");
        dictionary
            .admit(
                &first,
                NormalizedItem {
                    template: first.clone(),
                    slots: Vec::new(),
                },
                &[],
                1,
            )
            .unwrap();
        dictionary.next_surface = u32::MAX;
        let second = Item::new("test", "second");
        assert!(
            dictionary
                .admit(
                    &second,
                    NormalizedItem {
                        template: second.clone(),
                        slots: Vec::new(),
                    },
                    &[],
                    2,
                )
                .is_none()
        );
        assert_eq!(dictionary.templates.len(), 1);
        assert_eq!(dictionary.surfaces.len(), 1);
        assert!(dictionary.template_id(&second).is_none());
    }
}
