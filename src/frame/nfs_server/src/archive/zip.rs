//! ZIP provider (archive TODO T1/T2): a cloneable positional reader over the
//! outer file, the central directory walked on its own (raw names, flags,
//! duplicates the `zip` crate folds together), and member reads through the
//! crate by index.

use super::names::{self, RawName};
use crate::error::*;
use serde_json::json;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zip::result::ZipError;
use zip::ZipArchive;

/// What members are read with (D5). LZMA is left out: the crate's decoder fails on real samples (T8).
pub const METHODS: &[(u16, &str)] = &[(0, "stored"), (8, "deflate"), (9, "deflate64"), (12, "bzip2"), (93, "zstd"), (95, "xz")];
/// What `hello.archive` advertises: the methods decoded from samples (T8; no tool here writes Deflate64).
pub const VERIFIED_METHODS: &[&str] = &["stored", "deflate", "bzip2", "zstd", "xz"];
pub const ENCRYPTION: &[&str] = &["zipcrypto", "aes"];

pub fn method_name(method: u16) -> String {
    match method {
        14 => "lzma".to_string(),
        _ => METHODS.iter().find(|(m, _)| *m == method).map(|(_, n)| n.to_string()).unwrap_or_else(|| format!("method-{method}")),
    }
}

/// What the outer file must still be when it is opened again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub ino: u64,
    pub dev: u64,
    pub size: u64,
    pub mtime: i64,
}

impl Stamp {
    pub fn of(meta: &std::fs::Metadata) -> Stamp {
        let id = crate::fsutil::file_id(meta);
        let n = crate::fsutil::node_meta(meta);
        Stamp { ino: id.ino, dev: id.dev, size: meta.len(), mtime: n.mtime }
    }
}

#[derive(Debug)]
pub struct StaleOuter;

impl std::fmt::Display for StaleOuter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the archive changed since it was indexed")
    }
}

impl std::error::Error for StaleOuter {}

pub fn is_stale(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<StaleOuter>())
}

/// The outer file behind an index. The descriptor closes when idle and is
/// reopened on demand; a reopened file must match the stamp it was indexed at.
#[derive(Debug)]
pub struct Source {
    path: PathBuf,
    stamp: Option<Stamp>,
    pub len: u64,
    slot: Mutex<Option<(Arc<File>, Instant)>>,
}

impl Source {
    pub fn open(path: PathBuf, stamp: Option<Stamp>) -> io::Result<Arc<Source>> {
        let file = File::open(&path)?;
        let meta = file.metadata()?;
        if let Some(s) = &stamp {
            if Stamp::of(&meta) != *s {
                return Err(io::Error::other(StaleOuter));
            }
        }
        Ok(Arc::new(Source { path, stamp, len: meta.len(), slot: Mutex::new(Some((Arc::new(file), Instant::now()))) }))
    }

    fn file(&self) -> io::Result<Arc<File>> {
        let mut slot = self.slot.lock().unwrap();
        if let Some((f, used)) = slot.as_mut() {
            *used = Instant::now();
            return Ok(f.clone());
        }
        let file = File::open(&self.path)?;
        let meta = file.metadata()?;
        if meta.len() != self.len || self.stamp.as_ref().is_some_and(|s| Stamp::of(&meta) != *s) {
            return Err(io::Error::other(StaleOuter));
        }
        let file = Arc::new(file);
        *slot = Some((file.clone(), Instant::now()));
        Ok(file)
    }

    /// Drops an idle descriptor (readers in flight keep their own).
    pub fn close_idle(&self, idle: Duration) {
        let mut slot = self.slot.lock().unwrap();
        if slot.as_ref().is_some_and(|(_, used)| used.elapsed() >= idle) {
            *slot = None;
        }
    }

    pub fn reader(self: &Arc<Self>) -> PosReader {
        PosReader { src: self.clone(), file: None, pos: 0 }
    }
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(f, buf, off)
}

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(f, buf, off)
}

/// `Read + Seek` by position over a shared descriptor; clones keep their own cursor.
#[derive(Debug, Clone)]
pub struct PosReader {
    src: Arc<Source>,
    file: Option<Arc<File>>,
    pos: u64,
}

impl Read for PosReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.file.is_none() {
            self.file = Some(self.src.file()?);
        }
        let n = read_at(self.file.as_ref().unwrap(), buf, self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for PosReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let next = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.src.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = next.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        Ok(self.pos)
    }
}

/// One central directory record.
#[derive(Debug, Clone)]
pub struct Member {
    pub name: RawName,
    pub method: u16,
    pub encrypted: bool,
    pub aes: bool,
    pub strong: bool,
    pub crc32: u32,
    pub compressed: u64,
    pub size: u64,
    pub header_offset: u64,
    pub mtime: i64,
    pub symlink: bool,
    /// Index in the `zip` crate (None when another record shares its name there).
    pub index: Option<usize>,
}

impl Member {
    pub fn supported(&self) -> NfsResult<()> {
        if self.strong {
            return Err(NfsError::new(ErrorCode::Unsupported, "PKWARE strong encryption is not supported").with("reason", json!("encryption")));
        }
        if !METHODS.iter().any(|(m, _)| *m == self.method) {
            return Err(NfsError::new(ErrorCode::Unsupported, format!("compression method {} is not supported", self.method))
                .with("reason", json!("method")));
        }
        Ok(())
    }

    /// Readable as a byte range of the outer file.
    pub fn direct(&self) -> bool {
        self.method == 0 && !self.encrypted
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_central_directory_bytes: u64,
    pub max_entries: u64,
}

pub fn corrupt(msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::ArchiveCorrupt, msg)
}

pub fn io_error(e: io::Error) -> NfsError {
    if is_stale(&e) {
        return stale("the archive changed; open it again");
    }
    match e.kind() {
        io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => corrupt(format!("archive data: {e}")),
        _ => NfsError::from(e),
    }
}

pub fn zip_error(e: ZipError) -> NfsError {
    match e {
        ZipError::Io(e) => io_error(e),
        ZipError::InvalidArchive(m) => corrupt(format!("invalid archive: {m}")),
        ZipError::InvalidPassword => NfsError::new(ErrorCode::PasswordInvalid, "the password does not open this member"),
        ZipError::UnsupportedArchive(m) if m == ZipError::PASSWORD_REQUIRED => NfsError::new(ErrorCode::PasswordRequired, "this member is encrypted"),
        ZipError::UnsupportedArchive(m) => NfsError::new(ErrorCode::Unsupported, format!("unsupported archive: {m}")).with("reason", json!("zip")),
        other => corrupt(format!("archive: {other}")),
    }
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn read_exact_at(r: &mut PosReader, at: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    r.seek(SeekFrom::Start(at))?;
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// `(entries, central directory size)` from the end records, before the crate parses anything.
fn end_records(r: &mut PosReader) -> io::Result<Option<(u64, u64)>> {
    let len = r.src.len;
    let tail_len = len.min(22 + 65535) as usize;
    if tail_len < 22 {
        return Ok(None);
    }
    let tail = read_exact_at(r, len - tail_len as u64, tail_len)?;
    let Some(at) = (0..=tail_len - 22).rev().find(|&i| le32(&tail, i) == 0x0605_4b50) else { return Ok(None) };
    let entries = le16(&tail, at + 10) as u64;
    let size = le32(&tail, at + 12) as u64;
    let offset = le32(&tail, at + 16) as u64;
    if entries != 0xFFFF && size != 0xFFFF_FFFF && offset != 0xFFFF_FFFF {
        return Ok(Some((entries, size)));
    }
    let eocd = len - tail_len as u64 + at as u64;
    if eocd < 20 {
        return Ok(Some((entries, size)));
    }
    let locator = read_exact_at(r, eocd - 20, 20)?;
    if le32(&locator, 0) != 0x0706_4b50 {
        return Ok(Some((entries, size)));
    }
    let record = read_exact_at(r, le64(&locator, 8), 56)?;
    if le32(&record, 0) != 0x0606_4b50 {
        return Ok(Some((entries, size)));
    }
    Ok(Some((le64(&record, 32), le64(&record, 40))))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// DOS date/time is the zipping machine's wall clock: read it in this server's time zone, as zip tools show it.
fn dos_time(date: u16, time: u16) -> i64 {
    let (y, mo, d) = (((date >> 9) & 0x7f) as i64 + 1980, ((date >> 5) & 0xf).clamp(1, 12) as i64, (date & 0x1f).max(1) as i64);
    let (h, mi, s) = ((time >> 11) as i64, ((time >> 5) & 0x3f) as i64, ((time & 0x1f) * 2) as i64);
    local_epoch(y, mo, d, h, mi, s).unwrap_or_else(|| days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s)
}

#[cfg(unix)]
fn local_epoch(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    // SAFETY: `tm` is a plain C struct fully initialized here; mktime only reads and normalizes it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = (y - 1900) as libc::c_int;
    tm.tm_mon = (mo - 1) as libc::c_int;
    tm.tm_mday = d as libc::c_int;
    tm.tm_hour = h as libc::c_int;
    tm.tm_min = mi as libc::c_int;
    tm.tm_sec = s as libc::c_int;
    tm.tm_isdst = -1;
    let t = unsafe { libc::mktime(&mut tm) };
    (t != -1).then_some(t as i64)
}

#[cfg(not(unix))]
fn local_epoch(_: i64, _: i64, _: i64, _: i64, _: i64, _: i64) -> Option<i64> {
    None
}

/// Walks the central directory records in `cd` (all of them, duplicates included).
fn walk(cd: &[u8], archive_offset: u64) -> NfsResult<Vec<Member>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 46 <= cd.len() && le32(cd, at) == 0x0201_4b50 {
        let made_by = le16(cd, at + 4);
        let flags = le16(cd, at + 8);
        let mut method = le16(cd, at + 10);
        let (time, date) = (le16(cd, at + 12), le16(cd, at + 14));
        let crc32 = le32(cd, at + 16);
        let mut compressed = le32(cd, at + 20) as u64;
        let mut size = le32(cd, at + 24) as u64;
        let (name_len, extra_len, comment_len) = (le16(cd, at + 28) as usize, le16(cd, at + 30) as usize, le16(cd, at + 32) as usize);
        let external = le32(cd, at + 38);
        let mut header_offset = le32(cd, at + 42) as u64;
        let end = at + 46 + name_len + extra_len + comment_len;
        if end > cd.len() {
            return Err(corrupt("truncated central directory"));
        }
        let raw = cd[at + 46..at + 46 + name_len].to_vec();
        let extra = &cd[at + 46 + name_len..at + 46 + name_len + extra_len];
        let mut unicode = (flags & (1 << 11) != 0).then(|| String::from_utf8_lossy(&raw).into_owned());
        let mut name_raw = raw;
        let mut mtime = dos_time(date, time);
        let mut aes = false;
        let mut e = 0usize;
        while e + 4 <= extra.len() {
            let (kind, len) = (le16(extra, e), le16(extra, e + 2) as usize);
            let body = &extra[e + 4..(e + 4 + len).min(extra.len())];
            match kind {
                0x0001 => {
                    let mut p = 0;
                    let mut take = |v: &mut u64, wide: bool| {
                        if (wide || body.len() >= 24) && p + 8 <= body.len() {
                            *v = le64(body, p);
                            p += 8;
                        }
                    };
                    let (s, c, h) = (size == 0xFFFF_FFFF, compressed == 0xFFFF_FFFF, header_offset == 0xFFFF_FFFF);
                    take(&mut size, s);
                    take(&mut compressed, c);
                    take(&mut header_offset, h);
                }
                0x9901 if body.len() >= 7 => {
                    aes = true;
                    method = le16(body, 5);
                }
                0x5455 if !body.is_empty() && body[0] & 1 != 0 && body.len() >= 5 => {
                    mtime = i32::from_le_bytes(body[1..5].try_into().unwrap()) as i64;
                }
                0x7075 if body.len() > 5 && body[0] == 1 => {
                    if let Ok(s) = std::str::from_utf8(&body[5..]) {
                        unicode = Some(s.to_string());
                        name_raw = body[5..].to_vec();
                    }
                }
                _ => {}
            }
            e += 4 + len;
        }
        let symlink = made_by >> 8 == 3 && (external >> 16) & 0o170000 == 0o120000;
        out.push(Member {
            name: RawName { raw: name_raw, unicode },
            method,
            encrypted: flags & 1 != 0,
            aes,
            strong: flags & (1 << 6) != 0,
            crc32,
            compressed,
            size,
            header_offset: header_offset + archive_offset,
            mtime,
            symlink,
            index: None,
        });
        at = end;
    }
    Ok(out)
}

/// The parsed archive: the crate's archive (cloned per request; clones share
/// its metadata and the descriptor) and every central directory record.
#[derive(Debug)]
pub struct Parsed {
    pub zip: ZipArchive<PosReader>,
    pub members: Vec<Member>,
    pub naming: names::Naming,
}

pub fn parse(src: &Arc<Source>, limits: Limits) -> NfsResult<Parsed> {
    let mut reader = src.reader();
    let quota = |what: &str, limit: u64| NfsError::new(ErrorCode::QuotaExceeded, format!("the archive's {what} exceeds the limit")).with("limit", json!(limit));
    match end_records(&mut reader).map_err(io_error)? {
        Some((entries, _)) if entries > limits.max_entries => return Err(quota("entry count", limits.max_entries)),
        Some((_, size)) if size > limits.max_central_directory_bytes => return Err(quota("central directory", limits.max_central_directory_bytes)),
        Some(_) => {}
        None => return Err(corrupt("not a ZIP archive (no end of central directory record)")),
    }
    let zip = ZipArchive::new(reader.clone()).map_err(zip_error)?;
    if zip.len() as u64 > limits.max_entries {
        return Err(quota("entry count", limits.max_entries));
    }
    let cd_start = zip.central_directory_start();
    let cd_size = end_records(&mut reader).map_err(io_error)?.map(|(_, s)| s).unwrap_or(0).min(src.len.saturating_sub(cd_start));
    if cd_size > limits.max_central_directory_bytes {
        return Err(quota("central directory", limits.max_central_directory_bytes));
    }
    let cd = read_exact_at(&mut reader, cd_start, cd_size as usize).map_err(io_error)?;
    let mut members = walk(&cd, zip.offset())?;
    let key = |m: &Member| m.name.unicode.clone().unwrap_or_else(|| names::cp437(&m.name.raw));
    let mut uses: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in &members {
        *uses.entry(key(m)).or_default() += 1;
    }
    for m in members.iter_mut() {
        let k = key(m);
        if uses[&k] == 1 {
            m.index = zip.index_for_name(&k);
            if m.index.is_none() {
                log::warn!("archive: central directory record '{k}' has no crate index");
            }
        }
    }
    let flags = members.iter().map(|m| (m.symlink, m.index.is_some())).collect();
    let naming = names::Naming::new(members.iter().map(|m| m.name.clone()).collect(), flags);
    Ok(Parsed { zip, members, naming })
}

/// Where a stored, unencrypted member's bytes start in the outer file.
pub fn data_start(src: &Arc<Source>, m: &Member) -> NfsResult<u64> {
    let mut r = src.reader();
    let h = read_exact_at(&mut r, m.header_offset, 30).map_err(io_error)?;
    if le32(&h, 0) != 0x0403_4b50 {
        return Err(corrupt("bad local header"));
    }
    let start = m.header_offset + 30 + le16(&h, 26) as u64 + le16(&h, 28) as u64;
    if start.checked_add(m.compressed).is_none_or(|end| end > src.len) {
        return Err(corrupt("member data runs past the archive"));
    }
    Ok(start)
}

/// Opens a member for decoding (the crate checks a password against the
/// member's encryption header here: the quick check).
pub fn open<'a>(zip: &'a mut ZipArchive<PosReader>, m: &Member, password: Option<&[u8]>) -> NfsResult<zip::read::ZipFile<'a, PosReader>> {
    m.supported()?;
    let index = m.index.ok_or_else(|| NfsError::new(ErrorCode::AmbiguousEntry, "another record has the same name"))?;
    match (m.encrypted, password) {
        (true, None) => Err(NfsError::new(ErrorCode::PasswordRequired, "this member is encrypted")),
        (true, Some(p)) => zip.by_index_decrypt(index, p).map_err(zip_error),
        (false, _) => zip.by_index(index).map_err(zip_error),
    }
}

/// Feeds exactly the declared size of a decoding member to `sink`, then
/// checks the member ends there (CRC / AES authentication are checked at the
/// end by the crate). `limit` stops early (a partial range): no end check.
pub fn pump(mut r: impl Read, declared: u64, limit: Option<u64>, cancel: &AtomicBool, sink: &mut dyn FnMut(&[u8]) -> io::Result<bool>) -> NfsResult<bool> {
    let want = limit.map(|l| l.min(declared)).unwrap_or(declared);
    let mut left = want;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let n = r.read(&mut buf[..left.min(64 * 1024) as usize]).map_err(decode_error)?;
        if n == 0 {
            return Err(corrupt("the member is shorter than declared").with("reason", json!("decrypt_or_integrity")));
        }
        left -= n as u64;
        if !sink(&buf[..n]).map_err(NfsError::from)? {
            return Ok(false);
        }
    }
    if want < declared {
        return Ok(true);
    }
    let mut extra = [0u8; 1];
    match r.read(&mut extra).map_err(decode_error)? {
        0 => Ok(true),
        _ => Err(corrupt("the member is longer than declared").with("reason", json!("decrypt_or_integrity"))),
    }
}

fn decode_error(e: io::Error) -> NfsError {
    if is_stale(&e) {
        return stale("the archive changed; open it again");
    }
    corrupt(format!("the member does not decode: {e}")).with("reason", json!("decrypt_or_integrity"))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    pub fn build(path: &std::path::Path, entries: &[(&str, &[u8], zip::CompressionMethod)]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data, method) in entries {
            if name.ends_with('/') {
                w.add_directory(name.trim_end_matches('/'), SimpleFileOptions::default()).unwrap();
            } else {
                w.start_file(*name, SimpleFileOptions::default().compression_method(*method)).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
    }

    fn limits() -> Limits {
        Limits { max_central_directory_bytes: 1 << 20, max_entries: 1000 }
    }

    #[test]
    fn walks_records_and_reads_members() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.zip");
        let big = vec![b'x'; 200_000];
        build(&p, &[("docs/", b"", zip::CompressionMethod::Stored), ("docs/a.txt", b"hello", zip::CompressionMethod::Stored), ("big.bin", &big, zip::CompressionMethod::Deflated)]);
        let meta = std::fs::metadata(&p).unwrap();
        let src = Source::open(p.clone(), Some(Stamp::of(&meta))).unwrap();
        let mut parsed = parse(&src, limits()).unwrap();
        assert_eq!(parsed.members.len(), 3);
        let a = parsed.members.iter().find(|m| m.name.raw == b"docs/a.txt").unwrap().clone();
        assert!(a.direct());
        let start = data_start(&src, &a).unwrap();
        let mut r = src.reader();
        assert_eq!(read_exact_at(&mut r, start, 5).unwrap(), b"hello");
        let b = parsed.members.iter().find(|m| m.name.raw == b"big.bin").unwrap().clone();
        assert_eq!(method_name(b.method), "deflate");
        let mut out = Vec::new();
        let file = open(&mut parsed.zip, &b, None).unwrap();
        assert!(pump(file, b.size, None, &AtomicBool::new(false), &mut |c| {
            out.extend_from_slice(c);
            Ok(true)
        })
        .unwrap());
        assert_eq!(out, big);
        let mut lying = b.clone();
        lying.size = 1000;
        let file = open(&mut parsed.zip, &lying, None).unwrap();
        let e = pump(file, lying.size, None, &AtomicBool::new(false), &mut |_| Ok(true)).unwrap_err();
        assert_eq!(e.code, ErrorCode::ArchiveCorrupt);
        lying.size = 300_000;
        let file = open(&mut parsed.zip, &lying, None).unwrap();
        assert_eq!(pump(file, lying.size, None, &AtomicBool::new(false), &mut |_| Ok(true)).unwrap_err().code, ErrorCode::ArchiveCorrupt);
    }

    #[test]
    fn replaced_outer_is_stale_after_idle_close() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.zip");
        build(&p, &[("a.txt", b"one", zip::CompressionMethod::Stored)]);
        let src = Source::open(p.clone(), Some(Stamp::of(&std::fs::metadata(&p).unwrap()))).unwrap();
        let parsed = parse(&src, limits()).unwrap();
        let tmp = dir.path().join("b.zip");
        build(&tmp, &[("a.txt", b"two!", zip::CompressionMethod::Stored)]);
        std::fs::rename(&tmp, &p).unwrap();
        let mut held = parsed.zip.clone();
        assert!(held.by_index(0).is_ok(), "an open descriptor still reads the indexed bytes");
        src.close_idle(Duration::ZERO);
        let mut fresh = src.reader();
        let e = fresh.read(&mut [0u8; 4]).unwrap_err();
        assert!(is_stale(&e));
    }

    #[test]
    fn every_writable_method_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.zip");
        let data: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let methods = [zip::CompressionMethod::Stored, zip::CompressionMethod::Deflated, zip::CompressionMethod::Bzip2, zip::CompressionMethod::Zstd, zip::CompressionMethod::Xz];
        let entries: Vec<(String, zip::CompressionMethod)> = methods.iter().map(|m| (format!("{m:?}.bin"), *m)).collect();
        let refs: Vec<(&str, &[u8], zip::CompressionMethod)> = entries.iter().map(|(n, m)| (n.as_str(), data.as_slice(), *m)).collect();
        build(&p, &refs);
        let src = Source::open(p, None).unwrap();
        let mut parsed = parse(&src, limits()).unwrap();
        let mut seen = Vec::new();
        for m in parsed.members.clone() {
            let mut out = Vec::new();
            let file = open(&mut parsed.zip, &m, None).unwrap();
            pump(file, m.size, None, &AtomicBool::new(false), &mut |c| {
                out.extend_from_slice(c);
                Ok(true)
            })
            .unwrap();
            assert_eq!(out, data, "{}", method_name(m.method));
            seen.push(method_name(m.method));
        }
        assert_eq!(seen, vec!["stored", "deflate", "bzip2", "zstd", "xz"]);
        // LZMA (written by Python's zipfile): the crate's decoder fails on it, so it is refused up front.
        let lzma = hex::decode("504b03043f0002000e009db8705d15a2849924000000e0010000050000006c2e747874090405005d0000800000361e89dd7d4952e0f51c55829b13d810d60b0857ffffefca8000504b01023f033f0002000e009db8705d15a2849924000000e00100000500000000000000000000008001000000006c2e747874504b0506000000000100010033000000470000000000").unwrap();
        let p = dir.path().join("lzma.zip");
        std::fs::write(&p, lzma).unwrap();
        let src = Source::open(p, None).unwrap();
        let mut parsed = parse(&src, limits()).unwrap();
        let m = parsed.members[0].clone();
        assert_eq!(method_name(m.method), "lzma");
        let e = open(&mut parsed.zip, &m, None).err().unwrap();
        assert_eq!((e.code, e.details["reason"].as_str()), (ErrorCode::Unsupported, Some("method")));
    }

    #[test]
    fn entry_limit_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.zip");
        build(&p, &[("a", b"1", zip::CompressionMethod::Stored), ("b", b"2", zip::CompressionMethod::Stored)]);
        let src = Source::open(p, None).unwrap();
        let e = parse(&src, Limits { max_central_directory_bytes: 1 << 20, max_entries: 1 }).unwrap_err();
        assert_eq!(e.code, ErrorCode::QuotaExceeded);
        let not_zip = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(not_zip.path(), b"just text, not an archive").unwrap();
        let e = parse(&Source::open(not_zip.path().to_path_buf(), None).unwrap(), limits()).unwrap_err();
        assert_eq!(e.code, ErrorCode::ArchiveCorrupt);
    }
}
