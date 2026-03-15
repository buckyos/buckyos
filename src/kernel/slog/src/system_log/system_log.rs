use super::constants::*;
use super::flexi_log::*;
use super::log_config::*;
use super::target::*;
use crate::file::FileLogTarget;
use log::Log;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub struct SystemLogger {
    config: LogConfig,
    logger: FlexiLogger,
}

pub enum SystemLoggerCategory {
    Kernel,
    Service,
    App,
}

impl SystemLoggerCategory {
    pub fn as_str(&self) -> &str {
        match self {
            SystemLoggerCategory::Kernel => "kernel",
            SystemLoggerCategory::Service => "service",
            SystemLoggerCategory::App => "app",
        }
    }
}

pub struct SystemLoggerBuilder {
    log_root: PathBuf,
    name: String,
    config: LogConfig,
    targets: Vec<Box<dyn SystemLogTarget>>,
}

impl SystemLoggerBuilder {
    pub fn new(log_root: &Path, name: &str, _category: SystemLoggerCategory) -> Self {
        // simple_logger::SimpleLogger::default().init().unwrap();

        let log_dir = Self::get_log_dir(log_root, name);
        let config = LogConfig::new(log_dir);
        Self {
            log_root: log_root.to_path_buf(),
            name: name.to_string(),
            config,
            targets: vec![],
        }
    }

    pub fn directory(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config.set_log_dir(dir.into());
        self
    }

    pub fn level(mut self, level: &str) -> Self {
        self.config.global.set_level(level);
        self
    }

    pub fn console(mut self, level: &str) -> Self {
        self.config.global.set_console(level);
        self
    }

    pub fn enable_file_with_upload(mut self) -> Result<Self, String> {
        // First check if SystemLogTarget of this service already exists
        for target in &self.targets {
            if let Some(_) = target.as_any().downcast_ref::<FileLogTarget>() {
                // Found existing FileLogTarget
                return Ok(self);
            }
        }

        let log_dir = self.log_root.join(&self.name);
        let target = FileLogTarget::new(
            &log_dir,
            self.name.clone(),
            1024 * 1024 * 16, // 16 MB max file size
            1000,             // flush interval ms
        )?;

        let target = Box::new(target) as Box<dyn SystemLogTarget>;
        self.targets.push(target);

        Ok(self)
    }

    pub fn file(mut self, enable: bool) -> Self {
        self.config.global.file = enable;
        self
    }

    pub fn file_max_count(mut self, file_max_count: u32) -> Self {
        self.config.global.set_file_max_count(file_max_count);
        self
    }

    pub fn file_max_size(mut self, file_max_size: u64) -> Self {
        self.config.global.set_file_max_size(file_max_size);
        self
    }

    pub fn module(mut self, name: &str, level: Option<&str>, console_level: Option<&str>) -> Self {
        match Self::new_module(name, name, level, console_level) {
            Ok(config) => self.config.add_mod(config),
            Err(e) => {
                error!("invalid module log config for '{}': {}", name, e);
            }
        }

        self
    }

    pub fn target(mut self, target: Box<dyn SystemLogTarget>) -> Self {
        self.targets.push(target);
        self
    }

    pub fn disable_module(mut self, list: Vec<impl Into<String>>, level: LogLevel) -> Self {
        for name in list {
            let name = name.into();
            self.config.disable_module_log(&name, &level);
        }
        self
    }

    pub fn debug_info_flags(mut self, flags: u32) -> Self {
        self.config.global.set_debug_info_flags(flags);
        self
    }

    pub fn build(mut self) -> Result<SystemLogger, String> {
        self.config.disable_async_std_log();

        let logger = FlexiLogger::new(&self.config, self.targets)?;

        let ret = SystemLogger {
            config: self.config,
            logger,
        };

        Ok(ret)
    }

    pub fn get_log_dir(log_root: &Path, name: &str) -> PathBuf {
        assert!(!name.is_empty());

        let mut root = PathBuf::from(log_root);
        root.push(name);
        root
    }

    fn new_module(
        name: &str,
        file_name: &str,
        level: Option<&str>,
        console_level: Option<&str>,
    ) -> Result<LogModuleConfig, String> {
        let mut config = LogModuleConfig::new_default(name);
        if let Some(level) = level {
            config.level = LogLevel::from_str(level)?;
        }
        if let Some(level) = console_level {
            config.console = LogLevel::from_str(level)?;
        }

        config.file_name = Some(file_name.to_owned());
        Ok(config)
    }
}

impl Into<Box<dyn Log>> for SystemLogger {
    fn into(self) -> Box<dyn Log> {
        Box::new(self.logger) as Box<dyn Log>
    }
}

impl SystemLogger {
    pub fn start(self) -> Result<(), String> {
        let max_level = self.logger.get_max_level();
        let flags = self.config.global.get_debug_info_flags();

        if let Err(e) = log::set_boxed_logger(self.into()) {
            let msg = format!("call set_boxed_logger failed! {}", e);
            eprintln!("{}", msg);
            return Err(msg);
        }

        log::set_max_level(max_level.into());
        Self::display_debug_info(flags);
        Ok(())
    }

    pub fn display_debug_info(flags: LogDebugInfoFlags) {
        // Output environmental information to diagnose some environmental problems
        if flags.is_args_present() {
            for argument in std::env::args() {
                info!("arg: {}", argument);
            }
        }

        // info!("current exe: {:?}", std::env::current_exe());
        info!("current dir: {:?}", std::env::current_dir());

        if flags.is_env_present() {
            for (key, value) in std::env::vars() {
                info!("env: {}: {}", key, value);
            }
        }
    }

    pub fn flush() {
        log::logger().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::{SystemLoggerBuilder, SystemLoggerCategory};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn new_temp_log_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "buckyos/slog_system_logger_tests/{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_module_with_invalid_level_does_not_panic_and_keeps_buildable() {
        let dir = new_temp_log_dir("invalid_module_level");
        let logger = SystemLoggerBuilder::new(&dir, "svc", SystemLoggerCategory::Service)
            .module("svc_mod", Some("invalid_level"), Some("debug"))
            .build();
        assert!(logger.is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
