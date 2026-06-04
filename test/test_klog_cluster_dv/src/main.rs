include!("support.rs");

mod scenarios;

#[tokio::main]
async fn main() {
    if let Err(err) = scenarios::run().await {
        eprintln!("[klog-cluster-dv][error] {}", err);
        std::process::exit(1);
    }
}
