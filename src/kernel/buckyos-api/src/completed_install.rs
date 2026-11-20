use serde::{Deserialize, Serialize};
use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

pub const COMPLETED_INSTALL_FILE: &str = "etc/.last-buckyos-install.json";

#[derive(Debug, Deserialize, Serialize)]
enum Schema {
    #[serde(rename = "buckyos.completed_install.v1")]
    V1,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedInstall {
    schema: Schema,
    pub target_build: String,
    pub runtime_build: String,
    pub completed_at: u64,
}

impl CompletedInstall {
    pub fn new(completed_at: u64) -> Self {
        Self {
            schema: Schema::V1,
            target_build: target_build().to_owned(),
            runtime_build: runtime_build().to_owned(),
            completed_at,
        }
    }
}

/// Installation packages identify a build by version and build date. The runtime
/// also displays optional branch, commit and channel details.
fn package_build(version: &str) -> Option<&str> {
    let (base, date) = version.split_once("+build")?;
    let digits = date.bytes().take_while(u8::is_ascii_digit).count();
    (!base.is_empty() && digits > 0).then(|| &version[..version.len() - date.len() + digits])
}

pub fn target_build() -> &'static str {
    package_build(runtime_build()).expect("BuckyOS build version")
}

pub fn runtime_build() -> &'static str {
    let (version, _) = buckyos_kit::get_version()
        .rsplit_once(" (")
        .expect("BuckyOS release channel");
    version
}

fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        meta.is_symlink()
    }
}

/// Returns None until an installer has published a completion receipt at this root.
pub fn read(root: &Path) -> io::Result<Option<CompletedInstall>> {
    let path = root.join(COMPLETED_INSTALL_FILE);
    for dir in [root, path.parent().unwrap()] {
        match fs::symlink_metadata(dir) {
            Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid install directory {}", dir.display()),
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    let mut options = fs::File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || is_link(&meta) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a regular install receipt",
        ));
    }
    let receipt: CompletedInstall = serde_json::from_reader(file)?;
    if package_build(&receipt.runtime_build) != Some(receipt.target_build.as_str())
        || receipt.runtime_build.chars().any(char::is_whitespace)
        || receipt.completed_at == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid completed install build or timestamp",
        ));
    }
    Ok(Some(receipt))
}
