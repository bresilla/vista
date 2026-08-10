use crate::model::Stats;

#[cfg(feature = "surface-indexes")]
pub(crate) fn context_ratio(context_count: u64, surface: &Stats) -> f64 {
    context_count as f64 / surface.count.max(1) as f64
}

pub(crate) fn surface_ratio(surface: &Stats, template: &Stats, clock: u64) -> f64 {
    let frequency = surface.count as f64 / template.count.max(1) as f64;
    let age = clock.saturating_sub(surface.last_seen) as f64;
    frequency / (1.0 + age.ln_1p())
}
