use super::*;
use serde_json::json;

fn frame(input: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(input).unwrap();
    let mut frame = b"FCLT\x01\0\0\0".to_vec();
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend(body); frame
}

fn input() -> serde_json::Value {
    json!({"version":3,"values":{"kind":"array",
        "values":vec![json!({"kind":"undefined"}); 765]},
        "shared_worker_reply":{"kind":"undefined"}, "network_reply":{"kind":"undefined"}})
}

#[test]
fn exact_complete_collection_is_required() {
    assert_eq!(decode(&frame(&input())).unwrap().len(), 765);
    for size in [0, 764, 766] {
        let mut value = input();
        value["values"]["values"] = json!(vec![json!({"kind":"null"}); size]);
        assert!(decode(&frame(&value)).is_none());
    }
    for (key, value) in [("version", json!(2)), ("extra", json!(true))] {
        let mut data = input(); data[key] = value;
        assert!(decode(&frame(&data)).is_none());
    }
    let valid = frame(&input());
    let mut extra = valid.clone(); extra.push(0);
    let mut old = valid.clone(); old[..4].copy_from_slice(b"FSDK");
    let mut wrong_version = valid.clone(); wrong_version[4] = 2;
    for data in [valid[..valid.len()-1].to_vec(), extra, old, wrong_version,
        vec![0; LIMIT + 13]] { assert!(decode(&data).is_none()); }
}

#[test]
fn malformed_values_and_unbounded_data_cannot_encode() {
    for bad in [json!({"kind":"number","value":"inf"}),
        json!({"kind":"number","value":"01"}),
        json!({"kind":"number","value":"1e999"}),
        json!({"kind":"string","units":[65536]}),
        json!({"kind":"object","entries":[]}),
        json!({"kind":"null","raw":"private"}),
        json!({"kind":"string","units":vec![1;65537]}),
        json!({"kind":"array","values":vec![json!({"kind":"null"});2049]})] {
        let mut value = input(); value["values"]["values"][0] = bad;
        assert!(decode(&frame(&value)).is_none());
    }
    let mut value = input();
    for index in 0..2 {
        value["values"]["values"][index] = json!({"kind":"string","units":vec![1;32769]});
    }
    assert!(decode(&frame(&value)).is_none());
    let text = serde_json::to_string(&input()).unwrap().replace("\"version\":3",
        "\"version\":3,\"version\":3");
    let mut duplicate = b"FCLT\x01\0\0\0".to_vec();
    duplicate.extend_from_slice(&(text.len() as u32).to_be_bytes());
    duplicate.extend_from_slice(text.as_bytes());
    assert!(decode(&duplicate).is_none());
}

#[test]
fn missing_or_malformed_reply_cannot_produce_a_collection() {
    let mut missing = input();
    missing.as_object_mut().unwrap().remove("shared_worker_reply");
    assert!(decode(&frame(&missing)).is_none());
    for reply in [json!(null), json!({"kind":"object","entries":[]}),
        json!({"kind":"string","units":[65536]})] {
        let mut value = input(); value["shared_worker_reply"] = reply;
        assert!(decode(&frame(&value)).is_none());
    }
    let mut value = input();
    value["shared_worker_reply"] = json!({"kind":"string","units":vec![1;32768]});
    value["values"]["values"][0] = json!({"kind":"string","units":vec![1;32769]});
    assert!(decode(&frame(&value)).is_none());
}

#[test]
fn javascript_private_frame_preserves_values_for_native_encoding() {
    let script = r#"
      const {createCollector, writeCollection} = await import(process.argv[1]);
      const values = Array(765);
      values[0] = [undefined, null, -0, NaN, Infinity, -Infinity, '\ud800', true];
      values[242] = 'ignored-sdk-result';
      values[755] = 'sdk-owned-timing';
      const owner = createCollector(), reply = [2];
      reply[10] = 'native-version'; owner.sharedWorker(reply);
      owner.network(undefined);
      writeCollection(1, owner.collect(values));
    "#;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/offline_worker/collection.mjs");
    let output = std::process::Command::new(crate::test_support::node())
        .args(["--input-type=module", "--eval", script]).arg(path)
        .env_clear().output().unwrap();
    assert!(output.status.success());
    let actual = decode(&output.stdout).unwrap();
    let mut expected: Vec<_> = (0..765).map(|_| Value::Undefined).collect();
    expected[0] = Value::Array(vec![Value::Undefined, Value::Null,
        Value::Number(-0.0), Value::Number(f64::NAN), Value::Number(f64::INFINITY),
        Value::Number(f64::NEG_INFINITY), Value::String(vec![0xd800]), Value::Bool(true)]);
    expected[242] = Value::String("native-version".encode_utf16().collect());
    expected[755] = Value::String("sdk-owned-timing".encode_utf16().collect());
    v3::apply_network_result(&mut expected, Value::Undefined).unwrap();
    assert_eq!(v3::serialize(&actual), v3::serialize(&expected));
    assert_eq!(v3::encode_collected(&actual, 1700000000000),
        v3::encode_collected(&expected, 1700000000000));
    assert!(encode(output.stdout).unwrap().starts_with("cNz8rPYH|"));
}

#[test]
fn missing_invalid_and_oversized_network_results_refuse_encoding() {
    let mut missing = input();
    missing.as_object_mut().unwrap().remove("network_reply");
    assert!(decode(&frame(&missing)).is_none());
    for field in [40, 47, 18] {
        let mut row = vec![json!({"kind":"undefined"}); 48];
        row[field] = json!({"kind":"boolean","value":true});
        let mut value = input();
        value["network_reply"] = json!({"kind":"array",
            "values":[{"kind":"array","values":row}]});
        assert!(decode(&frame(&value)).is_none());
    }
    for reply in [json!(null), json!({"kind":"object","entries":[]}),
        json!({"kind":"array","values":[]}),
        json!({"kind":"array","values":[{"kind":"null"}]}),
        json!({"kind":"string","units":vec![1;65536]})] {
        let mut value = input(); value["network_reply"] = reply;
        value["values"]["values"][0] = json!({"kind":"string","units":[1]});
        assert!(decode(&frame(&value)).is_none());
    }
}
