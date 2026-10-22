use super::*;

#[path = "legacy_framing.rs"]
mod legacy;

#[derive(Debug, PartialEq, Eq)]
struct OwnedEvent { event: String, data: String, id: String }

fn save(event: Event<'_>, out: &mut Vec<OwnedEvent>) -> ControlFlow<()> {
    out.push(OwnedEvent {
        event: event.event.to_owned(),
        data: event.data.to_owned(),
        id: event.last_event_id.to_owned(),
    });
    ControlFlow::Continue(())
}

fn decoder() -> Decoder {
    Decoder::new(Limits { line_bytes: 4096, event_bytes: 8192 }).unwrap()
}

fn parse_chunks(chunks: &[&[u8]]) -> Vec<OwnedEvent> {
    let mut parser = decoder();
    let mut out = Vec::new();
    for chunk in chunks {
        parser.feed(chunk, |event| save(event, &mut out)).unwrap();
    }
    parser.finish();
    out
}

fn data(bytes: &[u8]) -> Vec<String> {
    parse_chunks(&[bytes]).into_iter().map(|event| event.data).collect()
}

#[test]
fn native_literal_space_baseline_misses_a_valid_terminal() {
    let payload = r#"{"type":"response.completed","response":{"output":[],"usage":{"input_tokens":7,"output_tokens":2}}}"#;
    let unspaced = format!("data:{payload}\n\n");
    let spaced = format!("data: {payload}\n\n");
    assert!(legacy::nativechain_data(&unspaced).is_empty());
    assert_eq!(legacy::nativechain_data(&spaced), vec![payload]);
    assert_eq!(data(unspaced.as_bytes()), vec![payload]);
    assert_eq!(data(spaced.as_bytes()), vec![payload]);
}

#[test]
fn server_lf_split_baseline_merges_distinct_crlf_events() {
    let text = "data:{\"type\":\"first\"}\r\n\r\ndata:{\"type\":\"second\"}\r\n\r\n";
    assert_eq!(legacy::server_data(text), vec!["{\"type\":\"first\"}\n{\"type\":\"second\"}"]);
    assert_eq!(data(text.as_bytes()), vec!["{\"type\":\"first\"}", "{\"type\":\"second\"}"]);
}

#[test]
fn every_two_chunk_boundary_preserves_lf_crlf_cr_and_utf8() {
    for eol in ["\n", "\r\n", "\r"] {
        let wire = format!("\u{feff}id: own{eol}event: response.completed{eol}data: fox 🦊 café{eol}{eol}data: next{eol}{eol}");
        let expected = vec![
            OwnedEvent { event: "response.completed".into(), data: "fox 🦊 café".into(), id: "own".into() },
            OwnedEvent { event: "message".into(), data: "next".into(), id: "own".into() },
        ];
        for cut in 0..=wire.len() {
            assert_eq!(parse_chunks(&[&wire.as_bytes()[..cut], &wire.as_bytes()[cut..]]), expected, "cut {cut}, eol {eol:?}");
        }
        let chunks: Vec<&[u8]> = wire.as_bytes().chunks(1).collect();
        assert_eq!(parse_chunks(&chunks), expected);
    }
}

#[test]
fn multiline_data_is_joined_at_logical_event_boundaries() {
    let wire = b"event: response.completed\ndata:{\"type\":\"response.completed\",\ndata: \"response\":{\"output\":[],\ndata:\ndata: \"usage\":{\"input_tokens\":7}}}\n\n";
    assert_eq!(data(wire), vec!["{\"type\":\"response.completed\",\n\"response\":{\"output\":[],\n\n\"usage\":{\"input_tokens\":7}}}"]);
}

#[test]
fn one_optional_ascii_space_is_removed_and_other_whitespace_survives() {
    let wire = b"data:  leading\ndata:\tkeep-tab\ndata: \tspace-then-tab\ndata:trailing  \n\n";
    assert_eq!(data(wire), vec![" leading\n\tkeep-tab\n\tspace-then-tab\ntrailing  "]);
}

#[test]
fn comments_unknown_fields_and_id_only_blocks_do_not_dispatch() {
    let wire = b": comment\nretry: 0\nData: wrong-case\n data: wrong-field\nid: private-id\nevent: must-reset\n\n";
    let mut parser = decoder();
    let mut out = Vec::new();
    parser.feed(wire, |event| save(event, &mut out)).unwrap();
    assert!(out.is_empty());
    parser.feed(b"data\n\n", |event| save(event, &mut out)).unwrap();
    assert_eq!(out, vec![OwnedEvent {
        event: "message".into(), data: "".into(), id: "private-id".into(),
    }]);
}

#[test]
fn event_name_is_last_value_and_id_persists_until_reset() {
    let wire = b"id: a\nevent: discarded\nevent: kept\ndata: first\n\nid: bad\0id\ndata: second\n\nid:\ndata: third\n\n";
    let out = parse_chunks(&[wire]);
    assert_eq!(out[0], OwnedEvent { event: "kept".into(), data: "first".into(), id: "a".into() });
    assert_eq!(out[1], OwnedEvent { event: "message".into(), data: "second".into(), id: "a".into() });
    assert_eq!(out[2], OwnedEvent { event: "message".into(), data: "third".into(), id: "".into() });
}

#[test]
fn only_initial_bom_is_removed_and_invalid_utf8_is_replaced() {
    let mut wire = b"\xef\xbb\xbfdata: start\n\n\xef\xbb\xbfdata: ignored\n\ndata: ".to_vec();
    wire.extend_from_slice(b"\xff\xef\xbb\xbf\n\n");
    let chunks: Vec<&[u8]> = wire.chunks(1).collect();
    let out = parse_chunks(&chunks);
    assert_eq!(out.iter().map(|event| event.data.as_str()).collect::<Vec<_>>(),
        vec!["start", "\u{fffd}\u{feff}"]);
}

#[test]
fn tools_reasoning_usage_and_effective_model_remain_opaque() {
    let payload = r#"{"type":"response.completed","response":{"model":"exact-model","service_tier":"priority","reasoning":{"effort":"max"},"usage":{"input_tokens":123,"output_tokens":0},"output":[{"type":"function_call","call_id":"call_A","namespace":"tools","name":"inspect","arguments":"{\"path\":\"one\\\\two\"}"},{"type":"custom_tool_call","call_id":"call_B","input":"literal\ntext"},{"type":"reasoning","encrypted_content":"opaque-private-history"}]}}"#;
    let wire = format!("data:{payload}\n\n");
    for size in 1..=17 {
        let chunks: Vec<&[u8]> = wire.as_bytes().chunks(size).collect();
        assert_eq!(parse_chunks(&chunks)[0].data, payload);
    }
}

#[test]
fn done_is_data_and_neither_terminal_nor_payload_is_fabricated() {
    assert_eq!(data(b"event: response.completed\n\n"), Vec::<String>::new());
    assert_eq!(data(b"data:[DONE]\n\n"), vec!["[DONE]"]);
    assert_eq!(data(b"data:[DONE]\n"), Vec::<String>::new());
    assert_eq!(data(b"data:\n\n"), vec![""]);
}

#[test]
fn eof_discards_unterminated_events_instead_of_claiming_success() {
    for suffix in ["data:unfinished", "data:unfinished\n", "data:unfinished\r", "data:unfinished\r\n"] {
        let wire = format!("data:complete\n\n{suffix}");
        assert_eq!(data(wire.as_bytes()), vec!["complete"]);
    }
    let mut parser = decoder();
    parser.feed(b"data: incomplete\n", |_| unreachable!()).unwrap();
    assert!(parser.buffered_bytes() > 0);
    parser.finish();
    parser.finish();
    assert_eq!(parser.buffered_bytes(), 0);
    assert_eq!(parser.feed(b"\n", |_| unreachable!()), Err(DecodeError::Finished));
}

#[test]
fn raw_line_limit_is_checked_before_append_and_failure_is_sticky() {
    let mut parser = Decoder::new(Limits { line_bytes: 8, event_bytes: 16 }).unwrap();
    parser.feed(b"data:123", |_| unreachable!()).unwrap();
    assert_eq!(parser.buffered_bytes(), 8);
    assert_eq!(parser.feed(b"4", |_| unreachable!()), Err(DecodeError::LineLimit));
    assert_eq!(parser.buffered_bytes(), 0);
    assert_eq!(parser.feed(b"\n\ndata:ok\n\n", |_| unreachable!()), Err(DecodeError::LineLimit));
    assert_eq!(DecodeError::LineLimit.to_string(), "sse: line exceeds byte limit");
}

#[test]
fn decoded_replacement_expansion_obeys_line_limit() {
    let mut parser = Decoder::new(Limits { line_bytes: 8, event_bytes: 16 }).unwrap();
    let mut out = Vec::new();
    parser.feed(b"data:\xff\n\n", |event| save(event, &mut out)).unwrap();
    assert_eq!(out[0].data, "\u{fffd}");
    let error = parser.feed(b"data:\xff\xff\n\n", |_| unreachable!());
    assert_eq!(error, Err(DecodeError::LineLimit));
    assert_eq!(parser.buffered_bytes(), 0);
}

#[test]
fn event_budget_counts_line_joins_name_and_persistent_id() {
    let limits = Limits { line_bytes: 16, event_bytes: 20 };
    let prefix = b"id: abcde\nevent: type\ndata: 1234567890\ndata\n";
    let mut parser = Decoder::new(limits).unwrap();
    parser.feed(prefix, |_| unreachable!()).unwrap();
    assert_eq!(parser.buffered_bytes(), 20);
    let mut out = Vec::new();
    parser.feed(b"\n", |event| save(event, &mut out)).unwrap();
    assert_eq!(out[0].data, "1234567890\n");
    assert_eq!(parser.buffered_bytes(), 5);

    let mut over = Decoder::new(limits).unwrap();
    over.feed(prefix, |_| unreachable!()).unwrap();
    assert_eq!(over.feed(b"data\n", |_| unreachable!()), Err(DecodeError::EventLimit));
    assert_eq!(over.buffered_bytes(), 0);
}

#[test]
fn cancellation_releases_state_and_cannot_resume() {
    let mut parser = decoder();
    parser.feed(b"id: account-a\ndata: unfinished", |_| unreachable!()).unwrap();
    assert!(parser.buffered_bytes() > 0);
    parser.cancel();
    parser.cancel();
    assert_eq!(parser.buffered_bytes(), 0);
    assert_eq!(parser.feed(b"\n\n", |_| unreachable!()), Err(DecodeError::Cancelled));
    parser.finish();
    assert_eq!(parser.feed(b"", |_| unreachable!()), Err(DecodeError::Cancelled));
}

#[test]
fn callback_break_stops_later_events_in_the_same_chunk() {
    let mut parser = decoder();
    let mut seen = Vec::new();
    let result = parser.feed(b"data:first\n\ndata:must-not-run\n\n", |event| {
        seen.push(event.data.to_owned());
        ControlFlow::Break(())
    });
    assert_eq!(result, Err(DecodeError::Cancelled));
    assert_eq!(seen, vec!["first"]);
    assert_eq!(parser.buffered_bytes(), 0);
}

#[test]
fn interleaved_accounts_do_not_share_partial_bytes_ids_or_events() {
    let mut a = decoder();
    let mut b = decoder();
    let mut out_a = Vec::new();
    let mut out_b = Vec::new();
    let wire_a = "id: only-a\nevent: tool\ndata: first 🦊\n\ndata: last-a\n\n";
    let wire_b = "id: only-b\r\nevent: usage\r\ndata: second ☃\r\n\r\ndata: last-b\r\n\r\n";
    for index in 0..wire_a.len().max(wire_b.len()) {
        if let Some(byte) = wire_a.as_bytes().get(index) {
            a.feed(&[*byte], |event| save(event, &mut out_a)).unwrap();
        }
        if let Some(byte) = wire_b.as_bytes().get(index) {
            b.feed(&[*byte], |event| save(event, &mut out_b)).unwrap();
        }
    }
    assert_eq!(out_a, vec![
        OwnedEvent { event: "tool".into(), data: "first 🦊".into(), id: "only-a".into() },
        OwnedEvent { event: "message".into(), data: "last-a".into(), id: "only-a".into() },
    ]);
    assert_eq!(out_b, vec![
        OwnedEvent { event: "usage".into(), data: "second ☃".into(), id: "only-b".into() },
        OwnedEvent { event: "message".into(), data: "last-b".into(), id: "only-b".into() },
    ]);
}

#[test]
fn long_stream_does_not_retain_prior_events() {
    let mut parser = Decoder::new(Limits { line_bytes: 32, event_bytes: 64 }).unwrap();
    let mut count = 0;
    for _ in 0..10_000 {
        parser.feed(b"data: one\n\n", |event| {
            assert_eq!(event.data, "one");
            count += 1;
            ControlFlow::Continue(())
        }).unwrap();
        assert_eq!(parser.buffered_bytes(), 0);
    }
    assert_eq!(count, 10_000);
    parser.finish();
    assert_eq!(parser.line.capacity() + parser.data.capacity()
        + parser.event.capacity() + parser.last_event_id.capacity(), 0);
}

#[test]
fn dispatch_waits_for_blank_line_and_crlf_never_dispatches_twice() {
    let mut parser = decoder();
    let mut out = Vec::new();
    parser.feed(b"data: one\r", |event| save(event, &mut out)).unwrap();
    assert!(out.is_empty());
    parser.feed(b"\n", |event| save(event, &mut out)).unwrap();
    assert!(out.is_empty());
    parser.feed(b"\r", |event| save(event, &mut out)).unwrap();
    assert_eq!(out.len(), 1);
    parser.feed(b"\n\n\r\n", |event| save(event, &mut out)).unwrap();
    assert_eq!(out.len(), 1);
}

#[test]
fn invalid_limits_are_rejected_without_allocating_a_decoder() {
    for limits in [
        Limits { line_bytes: 0, event_bytes: 1 },
        Limits { line_bytes: 1, event_bytes: 0 },
        Limits { line_bytes: 2, event_bytes: 1 },
    ] {
        assert!(matches!(Decoder::new(limits), Err(DecodeError::InvalidLimits)));
    }
}
