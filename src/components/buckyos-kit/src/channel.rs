use std::fmt::{Display, Formatter};
use std::str::FromStr;


#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BuckyOSChannel {
    Nightly,
    Beta,
    Stable,
}

impl FromStr for BuckyOSChannel {
    type Err = String;

    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let ret = match str {
            "nightly" => BuckyOSChannel::Nightly,
            "beta" => BuckyOSChannel::Beta,
            "stable" => BuckyOSChannel::Stable,
            _ => {
                log::warn!("unknown channel name {}, use default nightly channel", str);
                BuckyOSChannel::Nightly
            }
        };

        Ok(ret)
    }
}

impl Display for BuckyOSChannel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BuckyOSChannel::Nightly => write!(f, "nightly"),
            BuckyOSChannel::Beta => write!(f, "beta"),
            BuckyOSChannel::Stable => write!(f, "stable"),
        }
    }
}

impl BuckyOSChannel {
    fn get_ver(&self) -> u8 {
        match self {
            BuckyOSChannel::Nightly => 0,
            BuckyOSChannel::Beta => 1,
            BuckyOSChannel::Stable => 2,
        }
    }
}

pub fn get_version() -> &'static str {
    &VERSION
}

pub fn get_channel() -> &'static BuckyOSChannel {
    &CHANNEL
}

pub fn get_target() -> &'static str {
    &TARGET
}

fn get_version_impl() -> String {
    let channel_ver = get_channel().get_ver();
    format!("1.1.{}.{}-{} ({})", channel_ver, env!("VERSION"), get_channel(), env!("BUILDDATE"))
}

fn get_channel_impl() -> BuckyOSChannel {
    let channel_str = match std::env::var("CYFS_CHANNEL") {
        Ok(channel) => {
            info!("got channel config from CYFS_CHANNEL env: channel={}", channel);
            channel
        }
        Err(_) => {
            let channel = env!("CHANNEL").to_owned();
            info!("use default channel config: channel={}", channel);
            channel
        }
    };
    
    BuckyOSChannel::from_str(channel_str.as_str()).unwrap()
}

lazy_static::lazy_static! {
    static ref CHANNEL: BuckyOSChannel = get_channel_impl();
    static ref VERSION: String = get_version_impl();
    static ref TARGET: &'static str = env!("TARGET");
}
