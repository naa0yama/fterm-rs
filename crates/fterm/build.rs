#![allow(missing_docs)]

use std::process::Command;

/// Run `program` with `args` and return trimmed stdout, or `"unknown"` on failure.
fn capture_cmd(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| String::from("unknown"), |s| s.trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");

    let git_hash = capture_cmd("git", &["rev-parse", "--short", "HEAD"]);
    let rustc_version = capture_cmd("rustc", &["--version"]);

    println!("cargo:rustc-env=GIT_HASH={git_hash}");
    println!("cargo:rustc-env=RUSTC_VERSION_INFO={rustc_version}");
}
