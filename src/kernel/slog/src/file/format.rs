use crate::system_log::{SystemLogRecord, LogTimeHelper, LogLevel};
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
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.len() < 4 {
            let msg = format!("invalid log line format: {}", line);
            println!("{}", msg);
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
                println!("{}", msg);
                msg
            })?;
            let file_line_str = &content_part[1..end_pos];
            let file_line_parts: Vec<&str> = file_line_str.split(':').collect();
            if file_line_parts.len() != 2 {
                let msg = format!("invalid file and line format: {}", file_line_str);
                println!("{}", msg);
                return Err(msg);
            }
            let file = Some(file_line_parts[0].to_string());
            let line = Some(file_line_parts[1].parse::<u32>().map_err(|e| {
                let msg = format!("invalid line number: {}, {}", file_line_parts[1], e);
                println!("{}", msg);
                msg
            })?);
            let content = content_part[end_pos + 1..].trim().trim_end_matches('\n').to_string();
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
}