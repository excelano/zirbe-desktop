// Embeds the Windows application manifest through the MSVC linker, with no
// resource compiler, and does nothing on any other target.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#![forbid(unsafe_code)]

use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("packaging")
        .join("windows")
        .join("zirbe.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", manifest.display());

    // A build script runs on the host, so the target is read from the
    // environment rather than from cfg!.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }
    for arg in [
        "/MANIFEST:EMBED".to_string(),
        format!("/MANIFESTINPUT:{}", manifest.display()),
    ] {
        println!("cargo:rustc-link-arg-bin=zirbe={arg}");
    }
}
