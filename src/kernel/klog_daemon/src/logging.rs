use simplelog::{ColorChoice, Config as SimpleLogConfig, LevelFilter, TermLogger, TerminalMode};
use tracing_subscriber::{EnvFilter, fmt};

fn parse_level_filter(token: &str) -> Option<LevelFilter> {
    match token.trim().to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

fn simplelog_level_from_rust_log() -> LevelFilter {
    let raw = match std::env::var("RUST_LOG") {
        Ok(v) => v,
        Err(_) => return LevelFilter::Info,
    };
    if raw.trim().is_empty() {
        return LevelFilter::Info;
    }

    // Prefer global directive first, e.g. `warn,openraft=info`.
    for part in raw.split(',') {
        let p = part.trim();
        if p.is_empty() || p.contains('=') {
            continue;
        }
        if let Some(level) = parse_level_filter(p) {
            return level;
        }
    }

    // Fall back to first target directive level, e.g. `klog_daemon=warn`.
    for part in raw.split(',') {
        let p = part.trim();
        if let Some((_, rhs)) = p.split_once('=')
            && let Some(level) = parse_level_filter(rhs)
        {
            return level;
        }
    }

    LevelFilter::Info
}

pub fn init_logging() {
    let _ = TermLogger::init(
        simplelog_level_from_rust_log(),
        SimpleLogConfig::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    );

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use super::{LevelFilter, parse_level_filter};

    #[test]
    fn test_parse_level_filter_variants() {
        assert_eq!(parse_level_filter("warn"), Some(LevelFilter::Warn));
        assert_eq!(parse_level_filter("WARNING"), Some(LevelFilter::Warn));
        assert_eq!(parse_level_filter("info"), Some(LevelFilter::Info));
        assert_eq!(parse_level_filter("debug"), Some(LevelFilter::Debug));
        assert_eq!(parse_level_filter("trace"), Some(LevelFilter::Trace));
        assert_eq!(parse_level_filter("error"), Some(LevelFilter::Error));
        assert_eq!(parse_level_filter("off"), Some(LevelFilter::Off));
        assert_eq!(parse_level_filter("unknown"), None);
    }
}
