#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/native.m");
    println!("cargo:rerun-if-changed=src/native_windows.cpp");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build_windows();
        return;
    }
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
            "src/native.m",
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

fn build_windows() {
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ENV").as_deref(),
        Ok("msvc"),
        "Windows ComputerUse requires an MSVC target"
    );
    let (arch, expected_arch) = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "x86_64" => ("amd64", "x64"),
        "aarch64" => ("amd64_arm64", "arm64"),
        other => panic!("unsupported Windows architecture: {other}"),
    };
    let mut compiler_env = Vec::new();
    for name in ["VCINSTALLDIR", "VSCMD_ARG_TGT_ARCH"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var_os("VCINSTALLDIR").is_none()
        || env::var("VSCMD_ARG_TGT_ARCH").as_deref() != Ok(expected_arch)
    {
        let setup = if let Some(vc) = env::var_os("VCINSTALLDIR") {
            PathBuf::from(vc).join("Auxiliary/Build/vcvarsall.bat")
        } else {
            let installer = PathBuf::from(
                env::var_os("ProgramFiles(x86)").expect("ProgramFiles(x86) is unavailable"),
            )
            .join("Microsoft Visual Studio/Installer/vswhere.exe");
            let found = Command::new(installer)
                .args([
                    "-latest",
                    "-products",
                    "*",
                    "-requires",
                    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                    "-property",
                    "installationPath",
                    "-utf8",
                ])
                .output()
                .expect("Windows ComputerUse requires Visual Studio C++ build tools");
            assert!(
                found.status.success(),
                "cannot locate Visual Studio C++ build tools"
            );
            let installation = String::from_utf8(found.stdout).expect("invalid Visual Studio path");
            PathBuf::from(installation.trim()).join("VC/Auxiliary/Build/vcvarsall.bat")
        };
        assert!(
            setup.is_file(),
            "Visual Studio C++ build tools are unavailable"
        );
        let mut setup_command = Command::new("cmd.exe");
        setup_command.args(["/d", "/u", "/s", "/c"]);
        let setup_arguments = format!("\"\"{}\" {arch} >nul && set\"", setup.display());
        #[cfg(windows)]
        setup_command.raw_arg(setup_arguments);
        #[cfg(not(windows))]
        setup_command.arg(setup_arguments);
        let output = setup_command
            .output()
            .expect("cannot initialize Visual Studio C++ build tools");
        assert!(
            output.status.success(),
            "cannot initialize Visual Studio C++ build tools: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let environment = String::from_utf16(
            &output
                .stdout
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .expect("invalid Visual Studio environment");
        for line in environment.lines() {
            if let Some((name, value)) = line.split_once('=') {
                if !name.is_empty() {
                    compiler_env.push((name.to_owned(), value.to_owned()));
                }
            }
        }
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("computer_native.obj");
    let runtime = if env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
        .any(|feature| feature == "crt-static")
    {
        "/MT"
    } else {
        "/MD"
    };
    let status = Command::new("cl.exe")
        .envs(compiler_env.iter().cloned())
        .args([
            "/nologo",
            "/std:c++17",
            "/EHsc",
            "/O2",
            "/W4",
            "/WX",
            runtime,
            "/D_WIN32_WINNT=0x0A00",
            "/DWINVER=0x0A00",
            "/c",
            "src/native_windows.cpp",
        ])
        .arg(format!("/Fo{}", object.display()))
        .status()
        .expect("cannot run MSVC C++ compiler");
    assert!(
        status.success(),
        "cannot compile Windows ComputerUse bridge"
    );
    let status = Command::new("lib.exe")
        .envs(compiler_env)
        .arg("/nologo")
        .arg(format!(
            "/OUT:{}",
            out.join("computer_native.lib").display()
        ))
        .arg(object)
        .status()
        .expect("cannot archive Windows ComputerUse bridge");
    assert!(
        status.success(),
        "cannot archive Windows ComputerUse bridge"
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=computer_native");
    for library in [
        "user32", "gdi32", "gdiplus", "ole32", "advapi32", "wtsapi32", "shcore",
    ] {
        println!("cargo:rustc-link-lib={library}");
    }
}
