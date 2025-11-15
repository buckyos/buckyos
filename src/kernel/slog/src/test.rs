use crate::*;

fn test_log() {
    let logger = SystemLoggerBuilder::new(
        std::path::Path::new("/var/log/slog"),
        "test_service",
        SystemLoggerCategory::Service,
    )
    .level("info")
    .console("debug")
    .file(true)
    .build()
    .unwrap();

    logger.log_info("This is an info log from test_log.");
    logger.log_debug("This is a debug log from test_log.");
}   