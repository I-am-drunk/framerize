#![allow(dead_code, unused_imports)]
#[path = "../crates/framer-native/src/castle/mod.rs"] mod castle;
#[path = "../crates/framer-native/src/collection/mod.rs"] mod collection;
use castle::v3::{NetworkCandidate, NetworkObservation, Value, serialize};
use collection::NetworkCollator;
use serde_json::{Value as Json, json};
use std::io::{self, Read};

fn units(value: &Json) -> Vec<u16> {
    serde_json::from_value(value.clone()).expect("fixture UTF-16 units")
}

fn decode(value: &Json) -> Value {
    match value["kind"].as_str().expect("fixture kind") {
        "undefined" => Value::Undefined,
        "null" => Value::Null,
        "boolean" => Value::Bool(value["value"].as_bool().unwrap()),
        "string" => Value::String(units(&value["units"])),
        "number" => Value::Number(match value["value"].as_str().unwrap() {
            "NaN" => f64::NAN, "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY, number => number.parse().unwrap(),
        }),
        "object" => Value::Object(Vec::new()),
        "array" => Value::Array(Vec::new()),
        kind => panic!("unsupported fixture kind {kind}"),
    }
}

fn record(case: &Json, collator: &NetworkCollator) -> Json {
    let input = &case["record"];
    let candidates = input["candidates"].as_array().unwrap().iter().map(|row| {
        let values = row.as_array().unwrap();
        assert_eq!(values.len(), 7);
        NetworkCandidate {kind:decode(&values[0]), protocol:decode(&values[1]),
            address_class:decode(&values[2]), related_address_class:decode(&values[3]),
            port_class:decode(&values[4]), tcp_type:decode(&values[5]), component:decode(&values[6])}
    }).collect();
    let histories: Vec<Vec<Vec<u16>>> = serde_json::from_value(input["histories"].clone()).unwrap();
    let [connection_states, gathering_states, signaling_states] = histories.try_into().unwrap();
    let observation = NetworkObservation {candidates, connection_states, gathering_states,
        signaling_states, address:units(&input["address"]), related_address:units(&input["related"])};
    match collator.assemble(observation) {
        Ok(record) => json!({"bytes":serialize(&[record]).unwrap()}),
        Err(error) => json!({"error":format!("{error:?}")}),
    }
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let cases: Vec<Json> = serde_json::from_str(&input).unwrap();
    let output: Vec<_> = cases.iter().map(|case| {
        let collator = NetworkCollator::new(case["locale"].as_str().unwrap()).unwrap();
        if case.get("record").is_some() { return record(case, &collator); }
        let strings: Vec<Vec<u16>> = serde_json::from_value(case["strings"].clone()).unwrap();
        let mut signs = Vec::new();
        for left in &strings {
            for right in &strings {
                signs.push(collator.compare(left, right).map(|o| o as i8).ok());
            }
        }
        json!({"locale":case["locale"], "signs":signs})
    }).collect();
    println!("{}", serde_json::to_string(&output).unwrap());
}
