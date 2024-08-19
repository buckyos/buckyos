#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod etcd_control;

pub use system_config::SystemConfig;

#[cfg(test)]
mod tests {
    #[test]
    fn test_utility() {
        ()
    }
}
