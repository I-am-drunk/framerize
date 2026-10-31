//! Offline: read a worker FCLT frame from stdin, emit the wire token string.
//! Reuses the production castle v3 encoder and worker collection decoder.
#![allow(dead_code, unused_imports)]
#[path = "../crates/framer-native/src/castle/mod.rs"] mod castle;
use std::io::{self, Read};

#[path = "../crates/framer-native/src/worker/process/worker_collection.rs"]
mod collection;

fn main() {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes).expect("stdin");
    match collection::encode(bytes) {
        Some(token) => println!("{}", token),
        None => { eprintln!("encode failed"); std::process::exit(1); }
    }
}
