fn main() {
    println!("cargo:rerun-if-env-changed=VERSION");
    println!("cargo:rerun-if-env-changed=CHANNEL");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!(
        "cargo:rustc-env=VERSION={}",
        std::env::var("VERSION").unwrap_or("0".to_owned())
    );
    println!(
        "cargo:rustc-env=CHANNEL={}",
        std::env::var("CHANNEL").unwrap_or("nightly".to_owned())
    );
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    println!(
        "cargo:rustc-env=BUILDDATE={}",
        ::chrono::Local::now().format("%y-%m-%d")
    );

    println!("cargo:rerun-if-changed=protos");
}
