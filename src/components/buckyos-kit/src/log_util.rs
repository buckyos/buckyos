use crate::get_buckyos_log_dir;
use simplelog::*;
use std::fs::File;

pub fn init_logging(app_name: &str,is_service:bool) {
    // get log level in env RUST_LOG, default is info
    let log_level = std::env::var("BUCKY_LOG").unwrap_or_else(|_| "info".to_string());
    let log_level = log_level.parse().unwrap_or(log::LevelFilter::Info);
    // log_file in target dir, with pid
    let pid = std::process::id();
    let log_file;
    if is_service {
        log_file = get_buckyos_log_dir(app_name,is_service).join(format!("{}_{}.log", app_name,pid));
    } else {
        log_file = get_buckyos_log_dir(app_name,is_service).join(format!("{}.log", app_name));
    }
    std::fs::create_dir_all(log_file.parent().unwrap()).unwrap();

    let config = ConfigBuilder::new()
        .set_time_format_custom(format_description!("[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"))
        .set_location_level(LevelFilter::Debug)
        .build();

    let init_result = CombinedLogger::init(vec![
        TermLogger::new(
            log_level,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::options().append(true).create(true).open(log_file).unwrap(),
        ),
    ]);
    
    if init_result.is_err() {
        println!("Failed to init logging: {}", init_result.err().unwrap());
    }

}