use std::env;
use std::path::PathBuf;

fn main() {
    cc::Build::new()
        .cpp(true)
        // .define("_CRT_SECURE_NO_WARNINGS", None)
        // .define("WIN32", None)
        .define("_UNICODE", None)
        .define("UNICODE", None)
        .flag_if_supported("-std=c11")
        .file("src/entry.cpp")
        .file("src/TrayMenu.cpp")
        .file("src/process_kits.cpp")
        .compile("tray-controller");

    println!("cargo:rerun-if-changed=resource.rc");

    let out_dir = env::var("OUT_DIR").unwrap();
    let res_path = PathBuf::from(&out_dir).join("resource.res");

    let output = std::process::Command::new("windres")
        .args(&["src/tray_controller.rc", "-o"])
        .arg(res_path.to_str().unwrap())
        .status()
        .expect("Failed to compile resource file");
    assert!(output.success());

    println!("cargo:rustc-link-arg={}", res_path.display());

    println!("cargo:rustc-link-lib=user32");
    println!("cargo:rustc-link-lib=shell32");
}
