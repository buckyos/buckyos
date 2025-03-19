mod util;
mod etcd1;
mod gateway;

#[macro_use]
extern crate log;


fn main() {
    

    // use clap for args --gateway --etcd1 for test
    let matches = clap::Command::new("Gateway testr client")
        .arg(
            clap::Arg::new("gateway")
                .long("gateway")
                .takes_value(false)
                .help("Run test client run on gateway side"),
        )
        .arg(
            clap::Arg::new("etcd1")
                .long("etcd1")
                .takes_value(false)
                .help("Run test client run on etcd1 side"),
        )
        .get_matches();

    // init log
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();

    let rt = tokio::runtime::Runtime::new().unwrap();
    if matches.is_present("gateway") {
        info!("Will run gateway side client...");
        
        rt.block_on(gateway::run());
    } else if matches.is_present("etcd1") {
        info!("Will run etcd1 side client...");

        rt.block_on(etcd1::run());
    } else {
        error!("Please specify --gateway or --etcd1");
    }
}
