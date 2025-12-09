use cyfs_gateway::cyfs_gateway_main;

#[tokio::main]
async fn main() {
    println!("**** cyfs_gateway_main start!");
    cyfs_gateway_main().await;
}