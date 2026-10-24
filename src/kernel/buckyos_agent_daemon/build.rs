use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/computer/native.m");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported macOS architecture: {other}"),
    };
    let object = out.join("computer_native.o");
    let status = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang",
            "-fobjc-arc",
            "-fblocks",
            "-O2",
            "-Wall",
            "-Werror",
            "-mmacosx-version-min=14.0",
            "-arch",
            arch,
            "-c",
            "src/computer/native.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("macOS ComputerUse requires Xcode command line tools");
    assert!(status.success(), "cannot compile macOS ComputerUse bridge");
    let status = Command::new("xcrun")
        .args(["ar", "crs"])
        .arg(out.join("libcomputer_native.a"))
        .arg(object)
        .status()
        .expect("cannot archive macOS ComputerUse bridge");
    assert!(status.success(), "cannot archive macOS ComputerUse bridge");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=computer_native");
    println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
    for framework in [
        "Foundation",
        "AppKit",
        "ApplicationServices",
        "CoreGraphics",
        "ImageIO",
        "ScreenCaptureKit",
        "Carbon",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
