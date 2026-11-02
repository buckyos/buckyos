// A slow build: the build script works in steps of 10 seconds and says
// where it is after each one.
fn main() {
    for step in 1..=10 {
        std::thread::sleep(std::time::Duration::from_secs(10));
        println!("cargo:warning=beta: step {step}/10 done");
        eprintln!("beta: step {step}/10 done");
    }
}
