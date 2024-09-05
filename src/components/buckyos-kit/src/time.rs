

pub fn buckyos_get_unix_timestamp() -> u64 {
    let now = std::time::SystemTime::now();
    let unix_time = now.duration_since(std::time::UNIX_EPOCH).unwrap();
    unix_time.as_secs()
}