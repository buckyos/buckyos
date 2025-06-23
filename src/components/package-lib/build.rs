fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");
    let cargo_target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let cargo_target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let arch = if cargo_target_arch == "x86_64" { "amd64" } else { &cargo_target_arch };
    let os = if cargo_target_os == "macos" { "apple" } else { &cargo_target_os };
    // TODO: 这里的nightly也要通过某个环境变量指定
    let default_prefix = format!("nightly-{}-{}", os, arch);
    println!("cargo::rustc-env=PACKAGE_DEFAULT_PERFIX={}", default_prefix);
}