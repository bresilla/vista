use super::*;

use vista::{MatchInput, SimilarityMatcher};

#[derive(Clone, Copy)]
struct PackageNormalizer;

impl PackageNormalizer {
    fn slot(value: &str, prefix: &str) -> NormalizedItem {
        NormalizedItem {
            template: Item::new("command", format!("{prefix} {{pkg}}")),
            slots: vec![Feature::categorical("pkg", value)],
        }
    }
}

impl Normalizer for PackageNormalizer {
    fn normalize(&self, raw: &Item) -> NormalizedItem {
        for prefix in ["sudo apt install", "apt install"] {
            if let Some(package) = raw.value.strip_prefix(&format!("{prefix} ")) {
                return Self::slot(package, prefix);
            }
        }
        NormalizedItem {
            template: raw.clone(),
            slots: Vec::new(),
        }
    }

    fn render(&self, template: &Item, slots: &[Feature]) -> Option<Item> {
        let mut value = template.value.clone();
        for slot in slots {
            let Feature::Categorical { name, value: slot } = slot else {
                continue;
            };
            value = value.replace(&format!("{{{name}}}"), slot);
        }
        (!value.contains('{')).then(|| Item::new(template.namespace.clone(), value))
    }
}

fn shell(stream: u64, position: u64, value: &str) -> Observation {
    Observation {
        item: Item::new("command", value),
        stream: StreamId(stream),
        position: Position(position),
        timestamp: position as i64,
        context: Vec::new(),
        outcome: Vec::new(),
    }
}

fn package_predictor() -> Predictor {
    Predictor::builder(Config::default())
        .normalizer(PackageNormalizer)
        .matcher(SimilarityMatcher::default())
        .build()
}

#[test]
fn rendered_predictions_carry_the_source_arguments() {
    let mut predictor = package_predictor();
    predictor
        .replay([
            shell(1, 1, "sudo apt install fd"),
            shell(1, 2, "sudo apt install jq"),
            shell(1, 3, "sudo apt install bat"),
        ])
        .unwrap();

    let failed = Item::new("command", "apt install ripgrep");
    let rendered = predictor.predict_rendered(&query(1, 4, 3), &failed);

    assert_eq!(rendered[0].item.value, "sudo apt install ripgrep");
    assert_eq!(rendered[0].template.value, "sudo apt install {pkg}");
    assert!(
        !values(&rendered).iter().any(|value| value.contains("fd")),
        "historical arguments leaked into the rendered completion"
    );
}

#[test]
fn each_template_is_rendered_once() {
    let mut predictor = package_predictor();
    predictor
        .replay([
            shell(1, 1, "sudo apt install fd"),
            shell(1, 2, "sudo apt install jq"),
            shell(1, 3, "sudo apt install bat"),
            shell(1, 4, "sudo apt install ripgrep"),
        ])
        .unwrap();

    let rendered =
        predictor.predict_rendered(&query(1, 5, 8), &Item::new("command", "apt install hexyl"));
    let templates: Vec<_> = rendered
        .iter()
        .map(|prediction| prediction.template.value.as_str())
        .collect();
    let mut unique = templates.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(templates.len(), unique.len());
}

#[test]
fn template_matching_keeps_a_candidate_that_surface_matching_rejects() {
    let matcher = SimilarityMatcher::default();
    let failed = Item::new("command", "apt install ripgrep");
    let failed_template = Item::new("command", "apt install {pkg}");
    let candidate = Item::new("command", "sudo apt install fd");
    let candidate_template = Item::new("command", "sudo apt install {pkg}");

    let surface_only = matcher.score_match(MatchInput {
        partial: &failed.value,
        partial_template: None,
        candidate: &candidate,
        candidate_template: &candidate_template,
    });
    let with_template = matcher.score_match(MatchInput {
        partial: &failed.value,
        partial_template: Some(&failed_template),
        candidate: &candidate,
        candidate_template: &candidate_template,
    });

    assert!(surface_only.is_none(), "concrete arguments should dominate");
    assert!(with_template.is_some_and(|score| score > 0.6));
}

#[test]
fn unrenderable_templates_are_dropped() {
    let mut predictor = package_predictor();
    predictor
        .replay([
            shell(1, 1, "sudo apt install fd"),
            shell(1, 2, "cargo build --release"),
        ])
        .unwrap();

    let rendered = predictor.predict_rendered(
        &query(1, 3, 8),
        &Item::new("command", "apt install ripgrep"),
    );
    assert!(
        rendered
            .iter()
            .all(|prediction| !prediction.item.value.contains('{'))
    );
}

#[test]
fn identity_normalization_renders_templates_unchanged() {
    let mut predictor = Predictor::builder(Config::default())
        .normalizer(IdentityNormalizer)
        .build();
    predictor
        .replay([
            shell(1, 1, "build the project"),
            shell(1, 2, "run the tests"),
        ])
        .unwrap();

    let source = Item::new("command", "run the tests");
    let rendered = predictor.predict_rendered(&query(1, 3, 4), &source);
    assert!(!rendered.is_empty());
    assert!(
        rendered
            .iter()
            .all(|prediction| prediction.item == prediction.template)
    );
}

#[test]
fn default_matcher_and_render_keep_existing_behaviour() {
    let mut predictor = Predictor::new(Config::default());
    predictor
        .replay([
            shell(1, 1, "build the project"),
            shell(1, 2, "run the tests"),
        ])
        .unwrap();

    let mut partial_query = query(1, 3, 4);
    partial_query.partial = Some("run".into());
    assert_eq!(
        values(&predictor.predict(&partial_query)),
        vec!["run the tests"]
    );
}
