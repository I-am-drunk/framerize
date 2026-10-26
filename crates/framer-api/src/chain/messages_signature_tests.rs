use super::*;

#[test]
fn signature_delta_replaces_initial_value_and_retains_thinking() {
    let block = json!({"type":"thinking","thinking":"prefix","signature":"stale",
        "fixture_metadata":{"kept":true}});
    let values = events(block, vec![json!({"type":"thinking_delta","thinking":" suffix"}),
        json!({"type":"signature_delta","signature":"complete-signature"})], "end_turn");
    assert_eq!(parse(&encode(&values)).unwrap().content,
        vec![json!({"type":"thinking","thinking":"prefix suffix","signature":"complete-signature",
            "fixture_metadata":{"kept":true}})]);
}

#[test]
fn signature_replacement_retains_block_and_value_validation() {
    for (block, signature, expected) in [
        (json!({"type":"text","text":"answer"}), json!("signature"), "wrong block type"),
        (json!({"type":"thinking","thinking":"thought"}), json!(null), "non-text delta"),
        (json!({"type":"thinking","thinking":"thought","signature":42}),
            json!("signature"), "non-text block field"),
    ] {
        let values = events(block,
            vec![json!({"type":"signature_delta","signature":signature})], "end_turn");
        assert!(parse(&encode(&values)).unwrap_err().contains(expected));
    }
}
