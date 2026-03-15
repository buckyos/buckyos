use crate::system_log::{LogLevel, LogTimeHelper, SystemLogRecord};
use std::str::FromStr;

pub struct SystemLogRecordLineFormatter;

impl SystemLogRecordLineFormatter {
    pub fn format_record(record: &SystemLogRecord) -> String {
        if record.has_file_pos() {
            format!(
                "{} [{}] [{}] <{}:{}> {}\n",
                record.time_string(),
                record.level(),
                record.target(),
                record.file(),
                record.line(),
                record.content(),
            )
        } else {
            format!(
                "{} [{}] [{}] {}\n",
                record.time_string(),
                record.level(),
                record.target(),
                record.content(),
            )
        }
    }

    pub fn parse_record(line: &str) -> Result<SystemLogRecord, String> {
        // println!("Parsing log line: {}", line);
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.len() < 4 {
            let msg = format!("invalid log line format: {}", line);
            return Err(msg);
        }

        let time_str = parts[0];
        let time = LogTimeHelper::local_string_to_time(time_str)?;

        let level_str = parts[1].trim_matches(&['[', ']'][..]);
        let level = LogLevel::from_str(level_str)?;

        let target_str = parts[2].trim_matches(&['[', ']'][..]);

        // Check if there is file and line info
        let content_part = parts[3].trim();
        let (file, line, content) = if content_part.starts_with('<') {
            // Has file and line info
            let end_pos = content_part.find('>').ok_or_else(|| {
                let msg = format!("invalid log line format, missing '>': {}", line);
                msg
            })?;
            let file_line_str = &content_part[1..end_pos];
            let (file, line_num) = Self::parse_file_line(file_line_str)?;
            let file = Some(file);
            let line = Some(line_num);
            let content = content_part[end_pos + 1..]
                .trim()
                .trim_end_matches('\n')
                .to_string();
            (file, line, content)
        } else {
            // No file and line info
            (None, None, content_part.trim_end_matches('\n').to_string())
        };

        let record = SystemLogRecord {
            level,
            target: target_str.to_string(),
            time,
            file,
            line,
            content,
        };

        Ok(record)
    }

    fn parse_file_line(file_line_str: &str) -> Result<(String, u32), String> {
        // Split by the last `:` so Windows drive letters like `C:\...` are preserved.
        let mut parts = file_line_str.rsplitn(2, ':');
        let line_str = parts.next().unwrap_or_default().trim();
        let file_str = parts.next().unwrap_or_default().trim();

        if file_str.is_empty() || line_str.is_empty() {
            let msg = format!("invalid file and line format: {}", file_line_str);
            return Err(msg);
        }

        let line = line_str.parse::<u32>().map_err(|e| {
            format!(
                "invalid line number in file and line format: {}, {}",
                file_line_str, e
            )
        })?;

        Ok((file_str.to_string(), line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_and_parse_record() {
        let record = SystemLogRecord {
            level: LogLevel::Info,
            target: "test_target".to_string(),
            time: 1625079600,
            file: Some("test_file.rs".to_string()),
            line: Some(42),
            content: "This is a test log message.".to_string(),
        };

        let formatted = SystemLogRecordLineFormatter::format_record(&record);
        println!("Formatted log line: {}", formatted);
        let parsed = SystemLogRecordLineFormatter::parse_record(&formatted).unwrap();

        assert_eq!(record.level, parsed.level);
        assert_eq!(record.target, parsed.target);
        assert_eq!(record.time, parsed.time);
        assert_eq!(record.file, parsed.file);
        assert_eq!(record.line, parsed.line);
        assert_eq!(record.content, parsed.content);
    }

    #[test]
    fn test_parse_record_with_windows_file_path() {
        let record = SystemLogRecord {
            level: LogLevel::Warn,
            target: "win_target".to_string(),
            time: 1721000100000,
            file: Some(r"C:\work\buckyos\src\main.rs".to_string()),
            line: Some(128),
            content: "windows path test".to_string(),
        };

        let formatted = SystemLogRecordLineFormatter::format_record(&record);
        let parsed = SystemLogRecordLineFormatter::parse_record(&formatted).unwrap();
        assert_eq!(parsed.file.as_deref(), Some(r"C:\work\buckyos\src\main.rs"));
        assert_eq!(parsed.line, Some(128));
        assert_eq!(parsed.content, "windows path test");
        assert_eq!(parsed.level, LogLevel::Warn);
        assert_eq!(parsed.target, "win_target");
    }

    #[test]
    fn test_parse_record_without_file_position() {
        let record = SystemLogRecord {
            level: LogLevel::Info,
            target: "no_pos_target".to_string(),
            time: 1721000200000,
            file: None,
            line: None,
            content: "content without file pos".to_string(),
        };

        let formatted = SystemLogRecordLineFormatter::format_record(&record);
        let parsed = SystemLogRecordLineFormatter::parse_record(&formatted).unwrap();
        assert_eq!(parsed.file, None);
        assert_eq!(parsed.line, None);
        assert_eq!(parsed.content, "content without file pos");
        assert_eq!(parsed.level, LogLevel::Info);
        assert_eq!(parsed.target, "no_pos_target");
    }

    #[test]
    fn test_parse_record_file_path_with_extra_colon_uses_last_colon_for_line() {
        let record = SystemLogRecord {
            level: LogLevel::Error,
            target: "colon_target".to_string(),
            time: 1721000300000,
            file: Some("/var/log/archive:v1/app.rs".to_string()),
            line: Some(9),
            content: "colon in file path".to_string(),
        };

        let formatted = SystemLogRecordLineFormatter::format_record(&record);
        let parsed = SystemLogRecordLineFormatter::parse_record(&formatted).unwrap();
        assert_eq!(parsed.file.as_deref(), Some("/var/log/archive:v1/app.rs"));
        assert_eq!(parsed.line, Some(9));
    }

    #[test]
    fn test_parse_record_rejects_invalid_file_line_missing_colon() {
        let line = "2024-01-01_00:00:00.000_+00:00 [info] [test] <only_file> bad";
        let ret = SystemLogRecordLineFormatter::parse_record(line);
        assert!(ret.is_err());
        assert!(
            ret.unwrap_err()
                .contains("invalid file and line format: only_file")
        );
    }

    #[test]
    fn test_parse_record_rejects_invalid_file_line_non_numeric_line() {
        let line = "2024-01-01_00:00:00.000_+00:00 [info] [test] <C:\\a\\b.rs:xx> bad";
        let ret = SystemLogRecordLineFormatter::parse_record(line);
        assert!(ret.is_err());
        assert!(ret.unwrap_err().contains("invalid line number"));
    }

    #[test]
    fn test_parse_record_rejects_invalid_file_line_missing_file() {
        let line = "2024-01-01_00:00:00.000_+00:00 [info] [test] <:10> bad";
        let ret = SystemLogRecordLineFormatter::parse_record(line);
        assert!(ret.is_err());
        assert!(ret.unwrap_err().contains("invalid file and line format"));
    }

    #[test]
    fn test_parse_record_rejects_invalid_file_line_missing_line() {
        let line = "2024-01-01_00:00:00.000_+00:00 [info] [test] <C:\\a\\b.rs:> bad";
        let ret = SystemLogRecordLineFormatter::parse_record(line);
        assert!(ret.is_err());
        assert!(ret.unwrap_err().contains("invalid file and line format"));
    }
}
