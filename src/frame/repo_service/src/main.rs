mod repo_db;
mod service;

use log::error;

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
    if let Err(err) = runtime.block_on(service::run_service()) {
        error!("repo service exited with error: {err}");
        std::process::exit(1);
    }
}
