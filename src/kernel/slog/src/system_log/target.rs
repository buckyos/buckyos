use super::constants::*;
use log::{Log, Record};
use serde::{Serialize, Deserialize};
use chrono::offset::{Local, Utc};
use chrono::DateTime;

pub struct LogTimeHelper;

impl LogTimeHelper {
    pub fn now() -> u64 {
        chrono::Utc::now().timestamp_millis() as u64
    }

    pub fn time_to_local_string(time: u64) -> String {
        let datetime: DateTime<Utc> = DateTime::from_timestamp_millis(time as i64).unwrap_or(DateTime::default());
        let datetime: DateTime<Local> = DateTime::from(datetime);

        datetime.format("%Y-%m-%d_%H:%M:%S%.3f_%:z").to_string()
    }

    pub fn local_string_to_time(time_str: &str) -> Result<u64, String> {
        match DateTime::parse_from_str(time_str, "%Y-%m-%d_%H:%M:%S%.3f_%:z") {
            Ok(dt) => {
                let dt_utc: DateTime<Utc> = dt.with_timezone(&Utc);
                Ok(dt_utc.timestamp_millis() as u64)
            }
            Err(e) => {
                let msg = format!("failed to parse time string {}: {}", time_str, e);
                Err(msg)
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemLogRecord {
    pub level: LogLevel,
    pub target: String,
    pub time: u64,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub content: String,
}

impl SystemLogRecord {
    pub fn new(record: &Record) -> Self {
        let level: LogLevel = record.metadata().level().into();
        let target = record.metadata().target().to_owned();
        let time = LogTimeHelper::now();

        let content = format!("{}", record.args());

        Self {
            level,
            target,
            time,
            file: record.file().map(|v| v.to_owned()),
            line: record.line(),
            content,
        }
    }

    pub fn easy_log(level: LogLevel, content: String) -> Self {
        Self {
            level,
            time: LogTimeHelper::now(),
            target: "".to_string(),
            file: None,
            line: None,
            content,
        }
    }

    pub fn content(&self) -> &str {
        self.content.as_str()
    }

    pub fn level(&self) -> LogLevel {
        self.level
    }

    pub fn time(&self) -> u64 {
        self.time
    }

    pub fn time_string(&self) -> String {
        LogTimeHelper::time_to_local_string(self.time)
    }

    pub fn target(&self) -> &str {
        self.target.as_str()
    }

    pub fn has_file_pos(&self) -> bool {
        self.file.is_some() || self.line.is_some()
    }

    pub fn file(&self) -> &str {
        match &self.file {
            Some(f) => f.as_str(),
            _ => "",
        }
    }

    pub fn line(&self) -> u32 {
        match &self.line {
            Some(l) => *l,
            _ => 0,
        }
    }
}


impl std::fmt::Display for SystemLogRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // Covert timestamp in UTC milliseconds to local datetime

        let time_str = LogTimeHelper::time_to_local_string(self.time);

        write!(
            f,
            "[{}] {} [{}:{}] {}",
            time_str,
            self.level.to_string().to_uppercase(),
            self.file.as_deref().unwrap_or("<unnamed>"),
            self.line.unwrap_or(0),
            self.content,
        )
    }
}

pub trait SystemLogTarget: Send + Sync {
    fn log(&self, record: &SystemLogRecord);
}

pub struct ConsoleCyfsLogTarget {}

impl SystemLogTarget for ConsoleCyfsLogTarget {
    fn log(&self, record: &SystemLogRecord) {
        println!(">>>{}", record);
    }
}
