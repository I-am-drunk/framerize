//! Versioned builds use cargo_with_identity.py before Cargo checks its cache.
use std::env;

fn main() {
    println!("cargo:rerun-if-changed=../../scripts/build_identity.rs");
    println!("cargo:rerun-if-env-changed=FRAMERIZE_SOURCE_COMMIT");
    println!("cargo:rerun-if-env-changed=FRAMERIZE_SOURCE_DIRTY");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let identity = match (env::var("FRAMERIZE_SOURCE_COMMIT"), env::var("FRAMERIZE_SOURCE_DIRTY")) {
        (Ok(commit), Ok(dirty)) => {
            assert!(commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(dirty == "true" || dirty == "false");
            format!("{commit}{}", if dirty == "true" { ", dirty" } else { "" })
        }
        _ => "unverified source; build with scripts/cargo_with_identity.py".to_owned(),
    };
    println!("cargo:rustc-env=FRAMERIZE_BUILD_VERSION={version} ({identity})");
}
