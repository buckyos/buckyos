//! The occasion log: reading (tolerating an in-flight tail), cheap tail
//! reads for snapshots, and the archive manifest written by `compact`.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::MemoryOccasion;
use super::{AgentMemoryError, Result};

/// How the last bytes of a log ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Tail {
    #[default]
    Clean,
    /// A complete, verifiable envelope without its trailing newline.
    MissingNewline,
    /// Bytes after the last newline that do not form an envelope (an append
    /// interrupted mid-write); `offset` is where they start.
    Torn { offset: u64, len: u64 },
}

pub struct LogRead {
    pub occasions: Vec<MemoryOccasion>,
    pub tail: Tail,
}

pub fn occasion_digest(occasion: &MemoryOccasion) -> Result<String> {
    let mut clone = occasion.clone();
    clone.digest.clear();
    let bytes = serde_json::to_vec(&clone)?;
    Ok(format!("blake3:{}", blake3::hash(&bytes).to_hex()))
}

fn parse_line(path: &Path, lineno: usize, bytes: &[u8]) -> Result<MemoryOccasion> {
    let occ: MemoryOccasion = serde_json::from_slice(bytes)
        .map_err(|e| AgentMemoryError::Corrupted(format!("{}:{}: {e}", path.display(), lineno)))?;
    if occ.digest != occasion_digest(&occ)? {
        return Err(AgentMemoryError::Corrupted(format!(
            "{}:{}: digest mismatch for {}",
            path.display(),
            lineno,
            occ.occasion_id
        )));
    }
    Ok(occ)
}

/// Read a log file. A bad line in the middle is corruption; an unparsable
/// last segment without a newline is an in-flight or torn append and is
/// reported, not applied.
pub fn read_log(path: &Path) -> Result<LogRead> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogRead {
                occasions: Vec::new(),
                tail: Tail::Clean,
            })
        }
        Err(e) => return Err(e.into()),
    };
    let mut occasions = Vec::new();
    let mut start = 0usize;
    let mut lineno = 0usize;
    let mut tail = Tail::Clean;
    while start < bytes.len() {
        lineno += 1;
        match bytes[start..].iter().position(|b| *b == b'\n') {
            Some(rel) => {
                let line = &bytes[start..start + rel];
                if !line.iter().all(u8::is_ascii_whitespace) {
                    occasions.push(parse_line(path, lineno, line)?);
                }
                start += rel + 1;
            }
            None => {
                let rest = &bytes[start..];
                if rest.iter().all(u8::is_ascii_whitespace) {
                    break;
                }
                match parse_line(path, lineno, rest) {
                    Ok(occ) => {
                        occasions.push(occ);
                        tail = Tail::MissingNewline;
                    }
                    Err(_) => {
                        tail = Tail::Torn {
                            offset: start as u64,
                            len: rest.len() as u64,
                        };
                    }
                }
                break;
            }
        }
    }
    Ok(LogRead { occasions, tail })
}

/// The last complete (newline-terminated, non-blank) line of a file, read
/// backwards in blocks; an unterminated tail is ignored.
pub fn last_complete_line(path: &Path) -> Result<Option<Vec<u8>>> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let blank = |l: &[u8]| l.iter().all(u8::is_ascii_whitespace);
    let mut pos = f.metadata()?.len();
    let mut buf: Vec<u8> = Vec::new();
    const BLOCK: u64 = 8192;
    loop {
        if let Some(end_nl) = buf.iter().rposition(|b| *b == b'\n') {
            let mut stop = end_nl;
            loop {
                match buf[..stop].iter().rposition(|b| *b == b'\n') {
                    Some(prev) => {
                        if !blank(&buf[prev + 1..stop]) {
                            return Ok(Some(buf[prev + 1..stop].to_vec()));
                        }
                        stop = prev;
                    }
                    None if pos == 0 => {
                        return Ok((!blank(&buf[..stop])).then(|| buf[..stop].to_vec()));
                    }
                    None => break,
                }
            }
        } else if pos == 0 {
            return Ok(None);
        }
        let start = pos.saturating_sub(BLOCK);
        let mut chunk = vec![0u8; (pos - start) as usize];
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&buf);
        buf = chunk;
        pos = start;
    }
}

#[derive(Deserialize)]
struct SeqOnly {
    seq: u64,
}

/// Seq of the last committed line of a JSONL file whose lines carry `seq`
/// (the occasion log and perception files). Cheap: one backwards read.
pub fn tail_seq(path: &Path) -> Result<u64> {
    match last_complete_line(path)? {
        Some(line) => Ok(serde_json::from_slice::<SeqOnly>(&line)
            .map(|s| s.seq)
            .map_err(|e| {
                AgentMemoryError::Corrupted(format!("{}: tail line: {e}", path.display()))
            })?),
        None => Ok(0),
    }
}

/// Append one envelope line and fsync it (the commit point).
pub fn append_line(path: &Path, line: &[u8]) -> std::io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line)?;
    f.sync_all()
}

/// Drop a torn tail (never a committed line) before the next append.
pub fn truncate_tail(path: &Path, offset: u64) -> Result<()> {
    let f = OpenOptions::new().write(true).open(path)?;
    f.set_len(offset)?;
    f.sync_all()?;
    Ok(())
}

pub fn complete_missing_newline(path: &Path) -> Result<()> {
    append_line(path, b"\n")?;
    Ok(())
}

/// `.meta/archive/manifest.json`: the archived logs replay must find.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveManifest {
    pub archives: Vec<ArchiveEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub file: String,
    pub first_seq: u64,
    pub last_seq: u64,
    pub count: usize,
    pub digest: String,
}

impl ArchiveManifest {
    pub fn last_seq(&self) -> u64 {
        self.archives.iter().map(|a| a.last_seq).max().unwrap_or(0)
    }
}

pub fn read_manifest(dir: &Path) -> Result<ArchiveManifest> {
    let path = dir.join("manifest.json");
    match fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b)
            .map_err(|e| AgentMemoryError::Corrupted(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let stray = fs::read_dir(dir)
                .map(|rd| {
                    rd.flatten()
                        .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
                })
                .unwrap_or(false);
            if stray {
                return Err(AgentMemoryError::Corrupted(format!(
                    "{}: archived logs without a manifest",
                    dir.display()
                )));
            }
            Ok(ArchiveManifest::default())
        }
        Err(e) => Err(e.into()),
    }
}

pub fn file_digest(path: &Path) -> Result<String> {
    Ok(format!(
        "blake3:{}",
        blake3::hash(&fs::read(path)?).to_hex()
    ))
}

/// Paths in replay order (archives from the manifest, then the live log),
/// verifying each archive against its manifest entry.
pub fn log_paths(archive_dir: &Path, live: &Path, verify_digests: bool) -> Result<Vec<PathBuf>> {
    let manifest = read_manifest(archive_dir)?;
    let mut paths = Vec::new();
    for a in &manifest.archives {
        let p = archive_dir.join(&a.file);
        if !p.exists() {
            return Err(AgentMemoryError::Corrupted(format!(
                "archived log {} listed in the manifest is missing",
                a.file
            )));
        }
        if verify_digests && file_digest(&p)? != a.digest {
            return Err(AgentMemoryError::Corrupted(format!(
                "archived log {} does not match its manifest digest",
                a.file
            )));
        }
        paths.push(p);
    }
    paths.push(live.to_path_buf());
    Ok(paths)
}
