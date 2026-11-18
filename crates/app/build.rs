//! Build steps: the app icon (see `build/icon.rs`) and, below, MinGW import libraries.
//!
//! On the windows-gnu target, rustup's self-contained MinGW lacks some import
//! libraries (e.g. `libshlwapi.a`, needed by eframe's link-opening support).
//! Generate the missing ones with the `rust-lld` bundled with every toolchain.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "build/icon.rs"]
mod icon;

/// (dll name, exported symbols we need)
const IMPORT_LIBS: &[(&str, &[&str])] = &[("shlwapi", &["AssocQueryStringW"])];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/icon.rs");
    println!("cargo:rerun-if-changed=assets/icon.png");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let machine = (os == "windows").then(|| match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => 0x14c,
        Ok("aarch64") => 0xaa64,
        _ => 0x8664,
    });
    if let Err(e) = icon::generate(Path::new("assets/icon.png"), &out, machine) {
        panic!("app icon: {e}");
    }

    if os != "windows" || env != "gnu" {
        return;
    }
    if let Err(e) = generate() {
        println!("cargo:warning=could not generate MinGW import libraries ({e}); linking may fail without a full MinGW install");
    }
}

fn generate() -> Result<(), String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let host = std::env::var("HOST").map_err(|e| e.to_string())?;
    let out = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?);

    let sysroot = Command::new(&rustc).args(["--print", "sysroot"]).output().map_err(|e| e.to_string())?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    let lld = sysroot.join("lib").join("rustlib").join(&host).join("bin").join("rust-lld.exe");
    if !lld.is_file() {
        return Err(format!("{} not found", lld.display()));
    }

    let machine = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    for (dll, symbols) in IMPORT_LIBS {
        let def = out.join(format!("{dll}.def"));
        let mut text = format!("LIBRARY {dll}.dll\nEXPORTS\n");
        for s in *symbols {
            text.push_str(s);
            text.push('\n');
        }
        std::fs::write(&def, text).map_err(|e| e.to_string())?;
        let lib = out.join(format!("lib{dll}.a"));
        let status = Command::new(&lld)
            .args(["-flavor", "link", "/lib"])
            .arg(format!("/def:{}", def.display()))
            .arg(format!("/out:{}", lib.display()))
            .arg(format!("/machine:{machine}"))
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("rust-lld failed for {dll}"));
        }
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    Ok(())
}
