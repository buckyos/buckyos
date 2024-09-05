mod path;
mod process;
mod time;
pub use path::*;
pub use process::*;
pub use time::*;

mod test {
    use super::*;
    #[test]
    fn test_get_unix_timestamp() {
        let now = std::time::SystemTime::now();
        let unix_time = now.duration_since(std::time::UNIX_EPOCH).unwrap();
        assert_eq!(buckyos_get_unix_timestamp(), unix_time.as_secs());
    }
}