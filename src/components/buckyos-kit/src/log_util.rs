

use crate::get_buckyos_log_dir;
use simplelog::*;
use std::fs::File;

pub fn init_logging(service_name: &str) {
    // get log level in env RUST_LOG, default is info
    let log_level = std::env::var("BUCKY_LOG").unwrap_or_else(|_| "info".to_string());
    let log_level = log_level.parse().unwrap_or(log::LevelFilter::Info);
    // log_file in target dir, with pid
    let log_file = get_buckyos_log_dir(service_name).join(format!("{}.log", service_name,));
    std::fs::create_dir_all(log_file.parent().unwrap()).unwrap();

    let config = ConfigBuilder::new()
    .set_time_format_custom(format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"))
    .build();

    CombinedLogger::init(vec![
   
        TermLogger::new(
            log_level,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
   
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::create(log_file).unwrap(),
        ),
    ])
    .unwrap();

}