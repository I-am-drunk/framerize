//! Shared build script for the two application executables.
use std::{env, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(root).args(args)
        .output().expect("git is required for application build identity");
    assert!(output.status.success(), "git build identity command failed");
    String::from_utf8(output.stdout).expect("git output must be UTF-8")
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&manifest).parent().unwrap().parent().unwrap();
    let commit = git(root, &["rev-parse", "HEAD"]).trim().to_owned();
    assert!(commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()));
    let dirty = !git(root, &["status", "--porcelain", "--untracked-files=normal"])
        .trim().is_empty();
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let suffix = if dirty { ", dirty" } else { "" };
    println!("cargo:rustc-env=FRAMERIZE_BUILD_VERSION={version} ({commit}{suffix})");
    // Watch tracked source and Git metadata, including linked worktrees.
    for file in git(root, &["ls-files"]).lines() {
        println!("cargo:rerun-if-changed={}", root.join(file).display());
    }
    for name in ["HEAD", "index", "packed-refs"] {
        let file = git(root, &["rev-parse", "--path-format=absolute", "--git-path", name]);
        println!("cargo:rerun-if-changed={}", file.trim());
    }
    let head = git(root, &["rev-parse", "--symbolic-full-name", "HEAD"]);
    if head.trim().starts_with("refs/") {
        let file = git(root, &["rev-parse", "--path-format=absolute", "--git-path", head.trim()]);
        println!("cargo:rerun-if-changed={}", file.trim());
    }
}
