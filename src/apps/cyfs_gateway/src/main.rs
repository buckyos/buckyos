use cyfs_gateway::cyfs_gateway_main;


fn main() {
    println!("**** cyfs_gateway_main start!");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    rt.block_on(async {
        cyfs_gateway_main().await;
    });
}
