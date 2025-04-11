use backtrace::{Backtrace, BacktraceFrame};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::panic::PanicHookInfo;
use std::thread;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PanicInfo {
    pub msg: String,
    pub msg_with_symbol: String,
    pub hash: String,
}

impl PanicInfo {
    pub fn new(backtrace: Backtrace, info: &PanicHookInfo) -> Self {
        let backtrace_msg = Self::format_backtrace(&backtrace);
        let (msg, _) = Self::format_info(info, &backtrace_msg);

        let backtrace_msg = Self::format_backtrace_with_symbol(&backtrace);
        let (msg_with_symbol, hash) = Self::format_info(info, &backtrace_msg);

        let ret = Self {
            msg,
            msg_with_symbol,
            hash,
        };

        warn!("{}", ret.msg);
        warn!("{}", ret.msg_with_symbol);
        ret
    }

    fn format_info(info: &PanicHookInfo, backtrace: &str) -> (String, String) {
        let thread = thread::current();
        let thread = thread.name().unwrap_or("unnamed");

        let msg = match info.payload().downcast_ref::<&'static str>() {
            Some(s) => *s,
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => &**s,
                None => "[panic]",
            },
        };

        let unique_info;
        let msg = match info.location() {
            Some(location) => {
                unique_info = format!("{}:{}:{}", msg, location.file(), location.line());

                format!(
                    "thread '{}' panicked at '{}': {}:{}\n{}",
                    thread,
                    msg,
                    location.file(),
                    location.line(),
                    backtrace,
                )
            }
            None => {
                unique_info = format!("{}", msg);

                format!(
                    "thread '{}' panicked at '{}'\n{}",
                    thread,
                    msg,
                    backtrace
                )
            }
        };

        let mut sha256 = sha2::Sha256::new();
        sha256.update(unique_info.as_bytes());
        let ret = sha256.finalize();
        let hash = hex::encode(ret);

        // Only use the first 32 bytes
        let hash = hash[..32].to_owned();

        (msg, hash)
    }

    fn format_backtrace_with_symbol(backtrace: &Backtrace) -> String {
        format!("{:?}", backtrace)
    }

    fn format_backtrace(backtrace: &Backtrace) -> String {
        let frames: Vec<BacktraceFrame> = backtrace.clone().into();
        let mut values = Vec::new();
        for (i, frame) in frames.into_iter().enumerate() {
            if let Some(mod_addr) = frame.module_base_address() {
                let offset = frame.symbol_address() as isize - mod_addr as isize;
                values.push(format!("{}: {:#018x} {:#018p}", i, offset, mod_addr));
            } else {
                values.push(format!("{}: {:#018p}", i, frame.symbol_address()));
            }
        }

        values.join("\n")
    }

    /*
    fn calc_hash(backtrace: &Backtrace) -> String {
        let mut sha256 = sha2::Sha256::new();

        let frames: Vec<BacktraceFrame> = backtrace.clone().into();
        let mut values = Vec::new();
        for (i, frame) in frames.into_iter().enumerate() {
            if let Some(mod_addr) = frame.module_base_address() {
                let offset = frame.symbol_address() as isize - mod_addr as isize;
                values.push(format!("{}:{}", i, offset));
            } else {
                values.push(format!("{}:{:p}", i, frame.symbol_address()));
            }
        }

        let all = values.join("\n");

        sha256.input(all);
        let ret = sha256.result();
        let hash = hex::encode(ret);

        // Only use the first 32 bytes
        let hash = hash[..32].to_owned();

        info!("stack_hash=\n{}", hash);

        hash
    }
    */
}
