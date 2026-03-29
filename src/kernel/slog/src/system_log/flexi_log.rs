use super::constants::*;
use super::log_config::*;
use super::target::*;
use flexi_logger::{Cleanup, Criterion, DeferredNow, LogSpecification, Logger, Naming, Record};
use log::{Log, Metadata};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn console_format(
    w: &mut dyn std::io::Write,
    now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "[{}] {} [{:?}] [{}] {}",
        now.now().format("%Y-%m-%d %H:%M:%S%.6f"),
        record.level(),
        std::thread::current().id(),
        record.module_path().unwrap_or("<unnamed>"),
        //record.file().unwrap_or("<unnamed>"),
        //record.line().unwrap_or(0),
        &record.args()
    )
}

fn file_format(
    w: &mut dyn std::io::Write,
    now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "[{}] {} [{:?}] [{}:{}] {}",
        now.now().format("%Y-%m-%d %H:%M:%S%.6f %:z"),
        record.level(),
        std::thread::current().id(),
        record.file().unwrap_or("<unnamed>"),
        record.line().unwrap_or(0),
        &record.args()
    )
}

struct FlexiModuleLogger {
    config: LogModuleConfig,

    logger: Arc<Box<dyn Log>>,
}

impl FlexiModuleLogger {
    pub fn new(log_dir: &Path, config: &LogModuleConfig) -> Result<Self, String> {
        let mut config = config.clone();
        let logger = Self::new_logger(log_dir, &mut config)?;

        Ok(Self {
            config,
            logger: Arc::new(logger),
        })
    }

    pub fn clone_with_config(&self, config: &LogModuleConfig) -> Self {
        Self {
            config: config.clone(),
            logger: self.logger.clone(),
        }
    }

    fn check_level(&self, level: log::Level) -> bool {
        level as usize <= self.config.max_level() as usize
    }

    fn new_spec(config: &mut LogModuleConfig) -> LogSpecification {
        if let Ok(spec) = std::env::var("RUST_LOG") {
            if let Ok(ret) = LogSpecification::parse(&spec) {
                // When the log_level is read from the environment variable, the configuration needs to be updated in reverse
                let mut level = None;
                for m in ret.module_filters() {
                    // Check if the module name matches
                    match &m.module_name {
                        Some(name) => {
                            if *name == config.name {
                                level = Some(m.level_filter);
                                break;
                            }
                        }
                        None => {
                            if config.is_global_module() {
                                level = Some(m.level_filter);
                                break;
                            }
                        }
                    }
                }

                if let Some(level) = level {
                    config.level = level.into();
                    config.console = config.level;

                    println!(
                        "use RUST_LOG env for module: {} = {}",
                        config.name, config.level
                    );

                    return ret;
                }
            } else {
                println!(
                    "parse RUST_LOG env failed! module={}, spec={}",
                    config.name, spec
                );
            }
        }

        println!(
            "new logger: mod={}, level={}",
            config.name,
            config.max_level()
        );

        flexi_logger::LogSpecBuilder::from_module_filters(&[flexi_logger::ModuleFilter {
            module_name: None,
            level_filter: config.max_level().into(),
        }])
        .build()
    }

    fn new_logger(log_dir: &Path, config: &mut LogModuleConfig) -> Result<Box<dyn Log>, String> {
        println!(
            "new logger: dir={}, name={}, level={}, console={}",
            log_dir.display(),
            config.name,
            config.level,
            config.console,
        );

        let discriminant = if config.name == "global" {
            std::process::id().to_string()
        } else {
            let file_name = match &config.file_name {
                Some(v) => v.as_str(),
                None => config.name.as_str(),
            };
            format!("{}_{}", file_name, std::process::id())
        };

        let spec = Self::new_spec(config);
        let file_spec = flexi_logger::FileSpec::default()
            .directory(log_dir)
            .discriminant(discriminant)
            .suppress_timestamp();

        let mut logger = Logger::with(spec);

        if config.file {
            logger = logger
                .log_to_file(file_spec)
                .rotate(
                    Criterion::Size(config.file_max_size),
                    Naming::Numbers,
                    Cleanup::KeepLogFiles(config.file_max_count as usize),
                )
                .format_for_files(file_format);
        }

        if config.console != LogLevel::Off {
            logger = logger.duplicate_to_stderr(config.console.into());
            logger = logger.format_for_stderr(console_format);

            //#[cfg(feature = "colors")]
            //{
            //    logger = logger.format_for_stderr(cyfs_colored_default_format);
            //}
        }

        let (logger, _handle) = logger.build().map_err(|e| {
            let msg = format!("init logger failed! {}", e);
            println!("{}", msg);

            msg
        })?;

        Ok(logger)
    }
}

pub struct FlexiLogger {
    global_logger: FlexiModuleLogger,
    module_loggers: HashMap<String, FlexiModuleLogger>,
    max_level: LogLevel,

    targets: Vec<Box<dyn SystemLogTarget>>,
}

impl FlexiLogger {
    pub fn new(config: &LogConfig, targets: Vec<Box<dyn SystemLogTarget>>) -> Result<Self, String> {
        let global_logger = FlexiModuleLogger::new(&config.log_dir, &config.global)?;
        let mut max_level = global_logger.config.max_level();

        let mut module_loggers = HashMap::new();
        for (k, mod_config) in &config.modules {
            // Must use the level inside the logger
            let level;
            if mod_config.file_name.is_some() {
                if let Ok(logger) = FlexiModuleLogger::new(&config.log_dir, mod_config) {
                    level = logger.config.max_level();
                    println!("new logger mod with isolate file: {} {}", k, level);
                    module_loggers.insert(k.clone(), logger);
                } else {
                    continue;
                }
            } else {
                let logger = global_logger.clone_with_config(mod_config);
                level = logger.config.max_level();
                println!("new logger mod clone from global: {} {}", k, level);
                module_loggers.insert(k.clone(), logger);
            }

            if level > max_level {
                max_level = level;
            }
        }

        Ok(Self {
            max_level,
            global_logger,
            module_loggers,
            targets,
        })
    }

    pub fn get_max_level(&self) -> LogLevel {
        self.max_level
    }

    fn get_logger(&self, target: &str) -> &FlexiModuleLogger {
        let mod_name = match target.find("::") {
            Some(pos) => &target[..pos],
            None => target,
        };

        if let Some(item) = self.module_loggers.get(mod_name) {
            item
        } else {
            &self.global_logger
        }
    }
}

impl Log for FlexiLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let target = metadata.target();
        let logger = self.get_logger(target);
        logger.check_level(metadata.level())
    }

    fn log(&self, record: &Record) {
        let target = record.metadata().target();

        let logger = self.get_logger(target);
        if logger.check_level(record.metadata().level()) {
            //println!("will output");
            logger.logger.log(record);

            // If there are other targets, output to the targets
            if !self.targets.is_empty() {
                let record = SystemLogRecord::new(record);
                for target in &self.targets {
                    target.log(&record);
                }
            }
        }
    }

    fn flush(&self) {
        self.global_logger.logger.flush();
        for mod_config in self.module_loggers.values() {
            mod_config.logger.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn new_temp_log_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "buckyos/slog_flexi_tests/{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_flexi_module_logger_check_level_uses_max_of_file_and_console() {
        let dir = new_temp_log_dir("check_level");
        let mut config = LogModuleConfig::new_default("svc_level_check");
        config.file = false;
        config.level = LogLevel::Info;
        config.console = LogLevel::Debug;

        let logger = FlexiModuleLogger::new(&dir, &config).unwrap();
        assert!(logger.check_level(Level::Debug));
        assert!(logger.check_level(Level::Info));
        assert!(!logger.check_level(Level::Trace));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_flexi_logger_max_level_uses_module_max_level() {
        let dir = new_temp_log_dir("max_level");
        let mut config = LogConfig::new(dir.clone());
        config.global.level = LogLevel::Info;
        config.global.console = LogLevel::Warn;

        let mut module = LogModuleConfig::new_default("svc");
        module.file_name = None;
        module.file = false;
        module.level = LogLevel::Info;
        module.console = LogLevel::Trace;
        config.add_mod(module);

        let logger = FlexiLogger::new(&config, Vec::new()).unwrap();
        assert_eq!(logger.get_max_level(), LogLevel::Trace);

        let debug_meta = log::Metadata::builder()
            .level(Level::Debug)
            .target("svc::worker")
            .build();
        assert!(logger.enabled(&debug_meta));

        let trace_meta = log::Metadata::builder()
            .level(Level::Trace)
            .target("svc::worker")
            .build();
        assert!(logger.enabled(&trace_meta));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
