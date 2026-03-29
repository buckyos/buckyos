use flexi_logger::Duplicate;
use log::{Level, LevelFilter};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display},
    str::FromStr,
};

#[repr(usize)]
#[derive(
    Copy, Eq, PartialEq, PartialOrd, Ord, Clone, Debug, Hash, Serialize, Deserialize, Default,
)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    #[cfg_attr(not(debug_assertions), default)]
    Info = 3,
    #[cfg_attr(debug_assertions, default)]
    Debug = 4,
    Trace = 5,
}

impl TryFrom<u32> for LogLevel {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, String> {
        match value {
            0 => Ok(LogLevel::Off),
            1 => Ok(LogLevel::Error),
            2 => Ok(LogLevel::Warn),
            3 => Ok(LogLevel::Info),
            4 => Ok(LogLevel::Debug),
            5 => Ok(LogLevel::Trace),
            _ => Err("Invalid LogLevel value. Must be between 0 and 5.".to_string()),
        }
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let level = match *self {
            Self::Off => "off",
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        };
        write!(f, "{}", level)
    }
}

impl FromStr for LogLevel {
    type Err = String;

    /// Parse a string representation of an IPv4 address.
    fn from_str(level: &str) -> Result<LogLevel, String> {
        use LogLevel::*;

        let ret = match level {
            "off" => Off,
            "trace" => Trace,
            "debug" => Debug,
            "info" => Info,
            "warn" => Warn,
            "error" => Error,
            v => {
                let msg = format!("invalid log level: {}", v);
                println!("{}", msg);
                return Err(msg);
            }
        };

        Ok(ret)
    }
}

impl From<LogLevel> for Duplicate {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Trace => Duplicate::Trace,
            LogLevel::Debug => Duplicate::Debug,
            LogLevel::Info => Duplicate::Info,
            LogLevel::Warn => Duplicate::Warn,
            LogLevel::Error => Duplicate::Error,
            LogLevel::Off => Duplicate::None,
        }
    }
}

impl From<LogLevel> for LevelFilter {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Trace => LevelFilter::Trace,
            LogLevel::Debug => LevelFilter::Debug,
            LogLevel::Info => LevelFilter::Info,
            LogLevel::Warn => LevelFilter::Warn,
            LogLevel::Error => LevelFilter::Error,
            LogLevel::Off => LevelFilter::Off,
        }
    }
}

impl From<LevelFilter> for LogLevel {
    fn from(v: LevelFilter) -> Self {
        match v {
            LevelFilter::Trace => LogLevel::Trace,
            LevelFilter::Debug => LogLevel::Debug,
            LevelFilter::Info => LogLevel::Info,
            LevelFilter::Warn => LogLevel::Warn,
            LevelFilter::Error => LogLevel::Error,
            LevelFilter::Off => LogLevel::Off,
        }
    }
}

impl From<Level> for LogLevel {
    fn from(v: Level) -> Self {
        match v {
            Level::Trace => Self::Trace,
            Level::Debug => Self::Debug,
            Level::Info => Self::Info,
            Level::Warn => Self::Warn,
            Level::Error => Self::Error,
        }
    }
}
