use chrono::{DateTime, Datelike, Local, Timelike, Utc};

pub fn local_datetime(timestamp_ms: u64) -> DateTime<Local> {
    let secs = (timestamp_ms / 1000) as i64;
    let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nanos)
        .unwrap_or_else(Utc::now)
        .with_timezone(&Local)
}

pub fn format_local_timestamp(timestamp_ms: u64) -> String {
    local_datetime(timestamp_ms)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub fn format_local_compact_timestamp(timestamp_ms: u64) -> String {
    let dt = local_datetime(timestamp_ms);
    format!(
        "{}-{}-{} {:02}:{:02}:{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

pub fn format_local_date_ymd(timestamp_ms: u64) -> String {
    local_datetime(timestamp_ms).format("%Y-%m-%d").to_string()
}

pub fn format_local_hhmm(timestamp_ms: u64) -> String {
    local_datetime(timestamp_ms).format("%H:%M").to_string()
}

pub fn format_utc_datetime_as_local(value: DateTime<Utc>) -> String {
    value
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
