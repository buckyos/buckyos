use std::path::PathBuf;
use fern::Dispatch;
use chrono::Local;

fn get_log_dir(service:& str) -> PathBuf {
    let log_dir;
    #[cfg(target_os = "windows")]
    {
        log_dir = PathBuf::from(&format!("C:\\cyfs\\log\\{}", service));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "ios"))]
    {
        log_dir = PathBuf::from(&format!("/var/cyfs/log/{}", service));
    }

    // make sure the log dir exists
    if !log_dir.exists() {
        std::fs::create_dir_all(&log_dir).unwrap();
    }

    log_dir
}

pub fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    // get log level in env RUST_LOG, default is info
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let log_level = log_level.parse().unwrap_or(log::LevelFilter::Info);

    // log_file in target dir, with pid
    let log_file = get_log_dir("gateway").join(format!("gateway_{}.log", std::process::id()));

    Dispatch::new()
        .format(|out, message, record| {
            let now = Local::now();
            out.finish(format_args!(
                "{} [{}] {}",
                now.format("%Y-%m-%d_%H:%M:%S"),
                record.level(),
                message
            ))
        })
        .level(log_level)
        .chain(std::io::stdout())
        .chain(fern::log_file(log_file)?)
        .apply()?;
    Ok(())
}