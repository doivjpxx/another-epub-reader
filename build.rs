//! Build script: embeds the Windows icon/resources and publishes toolchain metadata.
//!
//! The About window shows the compiler version and the triple the binary was built for; both are
//! only knowable here, so they are forwarded to the crate as `EPUB_READER_*` env vars. Querying
//! `rustc` is strictly best effort: an About label is never worth failing a build over.

use std::process::Command;

fn main() {
    println!("cargo:rustc-env=EPUB_READER_RUSTC_VERSION={}", rustc_version());
    // `TARGET` is always set for build scripts, but stay total in case the script is run by hand.
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=EPUB_READER_TARGET={target}");

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        println!("cargo:rerun-if-changed=assets/app.rc");
        println!("cargo:rerun-if-changed=assets/app.ico");
        embed_resource::compile("assets/app.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}

/// The compiler version as `1.97.1`, taken from the first `rustc -vV` line
/// (`rustc 1.97.1 (8bab26f4f 2026-07-14)`), or `"unknown"` when the query fails.
fn rustc_version() -> String {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let Ok(output) = Command::new(rustc).arg("-vV").output() else {
        return "unknown".to_string();
    };
    if !output.status.success() {
        return "unknown".to_string();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("rustc "))
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".to_string())
}
