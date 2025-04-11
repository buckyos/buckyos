use std::env;
use std::path::PathBuf;

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    #[cfg(windows)]
    let platform_source = "win";
    #[cfg(target_os = "macos")]
    let platform_source = "macos";

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(format!("src/{}/entry.cpp", platform_source))
        .file(format!("src/{}/TrayMenu.cpp", platform_source));

    #[cfg(windows)]
    {
        build
            .define("_UNICODE", None)
            .define("UNICODE", None)
            .flag_if_supported("-std=c11")
            .compile("tray-controller");

        println!("cargo:rerun-if-changed=src/win/tray_controller.rc");

        let out_dir = env::var("OUT_DIR").unwrap();
        let res_path = PathBuf::from(&out_dir).join("resource.res");

        let output = std::process::Command::new("windres")
            .args(&["src/win/tray_controller.rc", "-o"])
            .arg(res_path.to_str().unwrap())
            .status()
            .expect("Failed to compile resource file");
        assert!(output.success());

        println!("cargo:rustc-link-arg={}", res_path.display());
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=shell32");
    }

    #[cfg(target_os = "macos")]
    {
        build
            .flag_if_supported("-std=c++11")
            .compile("tray-controller");
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    println!("This platform is not supported.");
}
