fn main() {
    cc::Build::new()
        .cpp(true)
        // .define("_CRT_SECURE_NO_WARNINGS", None)
        // .define("WIN32", None)
        .define("_UNICODE", None)
        .define("UNICODE", None)
        .flag_if_supported("-std=c11")
        .file("src/entry.cpp")
        .file("src/SystemState.cpp")
        .file("src/TrayMenu.cpp")
        .compile("tray-controller");

    println!("cargo:rustc-link-lib=user32");
    println!("cargo:rustc-link-lib=shell32");
}
