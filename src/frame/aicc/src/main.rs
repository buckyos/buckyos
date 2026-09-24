use log::error;

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("create AICC runtime");
    if let Err(error) = runtime.block_on(aicc::run_service()) {
        error!("AICC service exited with error: {error:#}");
        std::process::exit(1);
    }
}
