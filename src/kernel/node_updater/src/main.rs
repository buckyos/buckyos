mod preflight;
mod readers;
mod transaction;
#[cfg(windows)]
mod windows_permissions;

use anyhow::{ensure, Context, Result};
use buckyos_api::completed_install::{target_build, CompletedInstall, COMPLETED_INSTALL_FILE};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    Interactive,
    Silent,
}

#[derive(Parser)]
#[command(
    about = "Check and migrate local BuckyOS Desktop stores",
    after_help = "--version prints the target build. --identity prints its platform, SDK floor, and reader schemas as JSON."
)]
enum Command {
    Preflight(Args),
    Migrate(Args),
    /// Record that the installed service payload completed installation successfully.
    RecordInstall {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        target_build: String,
    },
}

#[derive(clap::Args)]
struct Args {
    #[arg(long)]
    root: PathBuf,
    #[arg(long)]
    target_build: String,
    #[arg(long)]
    mode: Mode,
    #[arg(long)]
    profile: String,
    #[arg(long)]
    security_root: PathBuf,
    #[arg(long)]
    result: PathBuf,
}

#[derive(Serialize)]
struct Report {
    status: &'static str,
    target_build: &'static str,
    phase: &'static str,
    recovery: &'static str,
    module: String,
    area: String,
    reason: String,
    source_build: Option<String>,
    changed_areas: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    incompatible_apps: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Self {
            status: "migration_failed",
            target_build: target_build(),
            phase: "validate",
            recovery: "unchanged",
            module: "node-updater".into(),
            area: String::new(),
            reason: String::new(),
            source_build: None,
            changed_areas: 0,
            incompatible_apps: Vec::new(),
        }
    }
}

// Resolve explicitly supplied roots, including a security directory that hasn't
// been created yet. All discovered descendants must stay inside these roots.
fn absolute_root(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "root must be absolute: {}",
        path.display()
    );
    ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "unexpected '..' in root"
    );
    for existing in path.ancestors() {
        if transaction::metadata(existing)?.is_some() {
            let canonical = existing.canonicalize()?;
            let remaining = path.strip_prefix(existing)?;
            // Joining an empty path can append a separator. A database must
            // retain its filename so a suffix such as "-wal" stays on the file.
            return Ok(if remaining.as_os_str().is_empty() {
                canonical
            } else {
                canonical.join(remaining)
            });
        }
    }
    anyhow::bail!("cannot resolve {}", path.display())
}

fn run(args: &Args, migrate: bool, report: &mut Report) -> Result<()> {
    // Until we know whether an earlier migrate was interrupted, keep services stopped.
    if migrate {
        report.recovery = "incomplete";
    }
    ensure!(
        args.target_build == target_build(),
        "target package {} requires a different updater ({} supplied)",
        args.target_build,
        target_build()
    );
    ensure!(
        matches!(args.profile.as_str(), "full" | "lite"),
        "unknown Desktop edition {}",
        args.profile
    );
    let root = absolute_root(&args.root)?;
    // An installer commonly inherits paths such as /var/... on macOS. Resolve
    // system aliases above the installation, while retaining checks on its own
    // security directory and any explicitly supplied external leaf.
    if let Ok(relative) = args.security_root.strip_prefix(&args.root) {
        transaction::validate_path(&root.join(relative))?;
    } else if let Some(parent) = args.security_root.parent() {
        let leaf = args
            .security_root
            .file_name()
            .context("invalid external security directory")?;
        transaction::validate_path(&absolute_root(parent)?.join(leaf))?;
    }
    let security = absolute_root(&args.security_root)?;
    ensure!(
        !root.starts_with(&security),
        "security directory cannot contain the installation root"
    );
    // The shared identity functions resolve these once per process.
    std::env::set_var("BUCKYOS_ROOT", &root);
    std::env::set_var("BUCKYOS_SECURITY_ROOT", &security);
    if migrate {
        report.area = root.display().to_string();
        report.phase = "recover";
        let store = transaction::Store::open(&root, &security)?;
        store.recover(target_build())?;
        report.recovery = "unchanged";
        report.phase = "prepare";
        let work = store.work()?;
        let prepared = readers::prepare(&root, &security, work.path(), &|message| {
            if args.mode == Mode::Silent {
                return Ok(false);
            }
            preflight::ask(message)
        })
        .map_err(|e| {
            report.module = e.module;
            report.area = e.area;
            if e.unsupported {
                report.status = "unsupported_schema";
            }
            anyhow::anyhow!(e.reason)
        })?;
        report.source_build = prepared.source_build;
        let changes = prepared.changes;
        report.phase = "apply";
        store
            .apply(&changes, work.path(), target_build())
            .map_err(|e| {
                if let Some(area) = e.cause.downcast_ref::<transaction::DataArea>() {
                    report.area = area.0.display().to_string();
                    if let Some(owner) = prepared.owners.get(&area.0) {
                        report.module = owner.to_string();
                    }
                }
                report.recovery = e.recovery;
                e.cause
            })?;
        report.changed_areas = changes.len();
        report.recovery = "committed";
    } else {
        report.phase = "preflight";
        report.module = "app-loader".into();
        report.area = "installed AppDocs".into();
        report.incompatible_apps = preflight::installed_conflicts(&root)?;
        preflight::decide(&report.incompatible_apps, args.mode, preflight::ask)?;
    }
    report.status = "success";
    Ok(())
}

fn record_install(root: &Path, expected: &str) -> Result<()> {
    ensure!(
        expected == target_build(),
        "installed updater {} does not match target {expected}",
        target_build()
    );
    let meta = std::fs::symlink_metadata(root)?;
    ensure!(
        meta.is_dir() && !meta.is_symlink(),
        "expected an installed root directory"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            meta.file_attributes() & 0x400 == 0,
            "reparse point at {}",
            root.display()
        );
    }
    let path = absolute_root(root)?.join(COMPLETED_INSTALL_FILE);
    transaction::validate_path(&path)?;
    if let Some(meta) = transaction::metadata(&path)? {
        ensure!(
            meta.is_file(),
            "expected a regular install receipt at {}",
            path.display()
        );
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    ensure!(now > 0, "invalid installation timestamp");
    transaction::write_install_receipt(&path, &CompletedInstall::new(now))
        .with_context(|| format!("record completed installation at {}", path.display()))
}

fn main() -> ExitCode {
    let flags: Vec<_> = std::env::args_os().skip(1).collect();
    if flags.len() == 1 && flags[0] == "--version" {
        println!("{}", target_build());
        return ExitCode::SUCCESS;
    }
    if flags.len() == 1 && flags[0] == "--identity" {
        if let Err(reason) = buckyos_api::check_app_sdk(None, buckyos_api::MINIMUM_APP_SDK) {
            eprintln!("Invalid release SDK floor: {reason}");
            return ExitCode::FAILURE;
        }
        let schemas: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/schemas.json")))
                .expect("built reader registry");
        let sources: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("OUT_DIR"),
            "/format_sources.json"
        )))
        .expect("linked format sources");
        let review: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/review.json")))
                .expect("reviewed registry");
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "amd64",
            a => a,
        };
        println!(
            "{}",
            json!({"target_build": target_build(), "platform": format!("{arch}-{}", std::env::consts::OS), "minimum_app_sdk": buckyos_api::MINIMUM_APP_SDK, "schemas": schemas, "format_sources": sources, "review": review})
        );
        return ExitCode::SUCCESS;
    }
    let (args, migrate) = match Command::parse() {
        Command::Migrate(args) => (args, true),
        Command::Preflight(args) => (args, false),
        Command::RecordInstall { root, target_build } => {
            if let Err(err) = record_install(&root, &target_build) {
                eprintln!("record-install: {err:#}");
                return ExitCode::FAILURE;
            }
            return ExitCode::SUCCESS;
        }
    };
    let mut report = Report::new();
    if let Err(err) = run(&args, migrate, &mut report) {
        report.reason = format!("{err:#}");
        eprintln!("{} ({}): {}", report.phase, report.area, report.reason);
    }
    if let Err(err) = transaction::write_json(&args.result, &report)
        .with_context(|| format!("write result {}", args.result.display()))
    {
        eprintln!("{err:#}");
        return ExitCode::FAILURE;
    }
    if report.status == "success" {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
