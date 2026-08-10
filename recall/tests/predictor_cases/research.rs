use super::*;

#[test]
fn research_export_is_deterministic_and_preserves_gaps() {
    let observations = [
        observation(1, 1, "a"),
        observation(1, 2, "b"),
        observation(1, 4, "a"),
    ];
    let export = ResearchExport::from_observations(observations).unwrap();
    assert_eq!(export.dictionary, vec![item("a"), item("b")]);
    assert_eq!(export.sequences, vec![vec![1, 2], vec![1]]);
    let mut spmf = Vec::new();
    export.write_spmf(&mut spmf).unwrap();
    assert_eq!(String::from_utf8(spmf).unwrap(), "1 -1 2 -1 -2\n1 -1 -2\n");
}

#[test]
fn research_export_orders_interleaved_sessions_chronologically() {
    let export = ResearchExport::from_observations([
        observation(2, 1, "stream-two"),
        observation(1, 1, "stream-one"),
        observation(2, 2, "stream-two-next"),
    ])
    .unwrap();
    assert_eq!(export.sequences, vec![vec![1, 3], vec![2]]);
}

#[test]
fn research_dictionary_uses_matching_positive_ids_and_escaped_fields() {
    let mut event = observation(1, 1, "line\nvalue\\tail");
    event.item.namespace = "name\twith\rcontrol".into();
    let export = ResearchExport::from_observations([event]).unwrap();
    let mut plain = Vec::new();
    let mut dictionary = Vec::new();
    export.write_plain(&mut plain).unwrap();
    export.write_dictionary(&mut dictionary).unwrap();

    assert_eq!(String::from_utf8(plain).unwrap(), "1\n");
    assert_eq!(
        String::from_utf8(dictionary).unwrap(),
        "1\tname\\twith\\rcontrol\tline\\nvalue\\\\tail\n"
    );
}
