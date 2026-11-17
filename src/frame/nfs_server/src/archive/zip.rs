//! ZIP provider (archive TODO T1/T2): the central directory walked on its
//! own (raw names, flags, duplicates the `zip` crate folds together), and
//! member reads through the crate by index.

use super::format::{Loc, Member};
use super::names::{self, RawName};
use super::source::*;
use crate::error::*;
use std::io;
use std::sync::Arc;
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

pub fn supported(method: u16, strong: bool) -> NfsResult<()> {
    if strong {
        return Err(unsupported("encryption", "PKWARE strong encryption is not supported"));
    }
    if !METHODS.iter().any(|(m, _)| *m == method) {
        return Err(unsupported("method", format!("compression method {method} is not supported")));
    }
    Ok(())
}

pub fn zip_error(e: ZipError) -> NfsError {
    match e {
        ZipError::Io(e) => io_error(e),
        ZipError::InvalidArchive(m) => corrupt(format!("invalid archive: {m}")),
        ZipError::InvalidPassword => NfsError::new(ErrorCode::PasswordInvalid, "the password does not open this member"),
        ZipError::UnsupportedArchive(m) if m == ZipError::PASSWORD_REQUIRED => NfsError::new(ErrorCode::PasswordRequired, "this member is encrypted"),
        ZipError::UnsupportedArchive(m) => unsupported("zip", format!("unsupported archive: {m}")),
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

/// `(entries, central directory size)` from the end records, before the crate parses anything.
fn end_records(r: &mut PosReader) -> io::Result<Option<(u64, u64)>> {
    let len = r.size();
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
        let strong = flags & (1 << 6) != 0;
        out.push(Member {
            name: RawName { raw: name_raw, unicode },
            size,
            compressed: Some(compressed),
            tag: format!("{crc32:08x}"),
            mtime,
            encrypted: flags & 1 != 0,
            hidden: symlink,
            readable: false,
            method: method_name(method),
            loc: Loc::Zip { method, strong, header_offset: header_offset + archive_offset, index: None },
        });
        at = end;
    }
    Ok(out)
}

/// Parses the central directory: the crate's archive (cloned per request;
/// clones share its metadata and the descriptor) and every record.
pub fn parse(src: &Arc<Source>, limits: Limits) -> NfsResult<(ZipArchive<PosReader>, Vec<Member>)> {
    let mut reader = src.reader();
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
            let found = zip.index_for_name(&k);
            if found.is_none() {
                log::warn!("archive: central directory record '{k}' has no crate index");
            }
            if let Loc::Zip { index, .. } = &mut m.loc {
                *index = found;
            }
            m.readable = found.is_some();
        }
    }
    Ok((zip, members))
}

/// Where a stored, unencrypted member's bytes start in the outer file.
pub fn data_start(src: &Arc<Source>, header_offset: u64, compressed: u64) -> NfsResult<u64> {
    let mut r = src.reader();
    let h = read_exact_at(&mut r, header_offset, 30).map_err(io_error)?;
    if le32(&h, 0) != 0x0403_4b50 {
        return Err(corrupt("bad local header"));
    }
    let start = header_offset + 30 + le16(&h, 26) as u64 + le16(&h, 28) as u64;
    if start.checked_add(compressed).is_none_or(|end| end > src.len) {
        return Err(corrupt("member data runs past the archive"));
    }
    Ok(start)
}

/// Opens a member for decoding (the crate checks a password against the
/// member's encryption header here: the quick check).
pub fn open<'a>(zip: &'a mut ZipArchive<PosReader>, m: &Member, password: Option<&[u8]>) -> NfsResult<zip::read::ZipFile<'a, PosReader>> {
    m.supported()?;
    let Loc::Zip { index, .. } = &m.loc else { return Err(internal("not a ZIP member")) };
    let index = index.ok_or_else(|| NfsError::new(ErrorCode::AmbiguousEntry, "another record has the same name"))?;
    match (m.encrypted, password) {
        (true, None) => Err(NfsError::new(ErrorCode::PasswordRequired, "this member is encrypted")),
        (true, Some(p)) => zip.by_index_decrypt(index, p).map_err(zip_error),
        (false, _) => zip.by_index(index).map_err(zip_error),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
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

    pub fn limits() -> Limits {
        Limits { max_central_directory_bytes: 1 << 20, max_entries: 1000, max_materialize_bytes: 1 << 30, max_decoder_memory: 256 << 20 }
    }

    fn header_offset(m: &Member) -> u64 {
        match m.loc {
            Loc::Zip { header_offset, .. } => header_offset,
            _ => unreachable!(),
        }
    }

    #[test]
    fn walks_records_and_reads_members() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.zip");
        let big = vec![b'x'; 200_000];
        build(&p, &[("docs/", b"", zip::CompressionMethod::Stored), ("docs/a.txt", b"hello", zip::CompressionMethod::Stored), ("big.bin", &big, zip::CompressionMethod::Deflated)]);
        let meta = std::fs::metadata(&p).unwrap();
        let src = Source::open(p.clone(), Some(Stamp::of(&meta))).unwrap();
        let (mut z, members) = parse(&src, limits()).unwrap();
        assert_eq!(members.len(), 3);
        let a = members.iter().find(|m| m.name.raw == b"docs/a.txt").unwrap().clone();
        assert!(a.direct());
        let start = data_start(&src, header_offset(&a), a.compressed.unwrap()).unwrap();
        let mut r = src.reader();
        assert_eq!(read_exact_at(&mut r, start, 5).unwrap(), b"hello");
        let b = members.iter().find(|m| m.name.raw == b"big.bin").unwrap().clone();
        assert_eq!(b.method, "deflate");
        let mut out = Vec::new();
        let file = open(&mut z, &b, None).unwrap();
        assert!(pump(file, b.size, None, &AtomicBool::new(false), &mut |c| {
            out.extend_from_slice(c);
            Ok(true)
        })
        .unwrap());
        assert_eq!(out, big);
        let mut lying = b.clone();
        lying.size = 1000;
        let file = open(&mut z, &lying, None).unwrap();
        let e = pump(file, lying.size, None, &AtomicBool::new(false), &mut |_| Ok(true)).unwrap_err();
        assert_eq!(e.code, ErrorCode::ArchiveCorrupt);
        lying.size = 300_000;
        let file = open(&mut z, &lying, None).unwrap();
        assert_eq!(pump(file, lying.size, None, &AtomicBool::new(false), &mut |_| Ok(true)).unwrap_err().code, ErrorCode::ArchiveCorrupt);
    }

    #[test]
    fn replaced_outer_is_stale_after_idle_close() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.zip");
        build(&p, &[("a.txt", b"one", zip::CompressionMethod::Stored)]);
        let src = Source::open(p.clone(), Some(Stamp::of(&std::fs::metadata(&p).unwrap()))).unwrap();
        let (z, _) = parse(&src, limits()).unwrap();
        let tmp = dir.path().join("b.zip");
        build(&tmp, &[("a.txt", b"two!", zip::CompressionMethod::Stored)]);
        std::fs::rename(&tmp, &p).unwrap();
        let mut held = z.clone();
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
        let (mut z, members) = parse(&src, limits()).unwrap();
        let mut seen = Vec::new();
        for m in members {
            let mut out = Vec::new();
            let file = open(&mut z, &m, None).unwrap();
            pump(file, m.size, None, &AtomicBool::new(false), &mut |c| {
                out.extend_from_slice(c);
                Ok(true)
            })
            .unwrap();
            assert_eq!(out, data, "{}", m.method);
            seen.push(m.method.clone());
        }
        assert_eq!(seen, vec!["stored", "deflate", "bzip2", "zstd", "xz"]);
        // LZMA (written by Python's zipfile): the crate's decoder fails on it, so it is refused up front.
        let lzma = hex::decode("504b03043f0002000e009db8705d15a2849924000000e0010000050000006c2e747874090405005d0000800000361e89dd7d4952e0f51c55829b13d810d60b0857ffffefca8000504b01023f033f0002000e009db8705d15a2849924000000e00100000500000000000000000000008001000000006c2e747874504b0506000000000100010033000000470000000000").unwrap();
        let p = dir.path().join("lzma.zip");
        std::fs::write(&p, lzma).unwrap();
        let src = Source::open(p, None).unwrap();
        let (mut z, members) = parse(&src, limits()).unwrap();
        let m = members[0].clone();
        assert_eq!(m.method, "lzma");
        let e = open(&mut z, &m, None).err().unwrap();
        assert_eq!((e.code, e.details["reason"].as_str()), (ErrorCode::Unsupported, Some("method")));
    }

    #[test]
    fn entry_limit_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.zip");
        build(&p, &[("a", b"1", zip::CompressionMethod::Stored), ("b", b"2", zip::CompressionMethod::Stored)]);
        let src = Source::open(p, None).unwrap();
        let e = parse(&src, Limits { max_entries: 1, ..limits() }).unwrap_err();
        assert_eq!(e.code, ErrorCode::QuotaExceeded);
        let not_zip = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(not_zip.path(), b"just text, not an archive").unwrap();
        let e = parse(&Source::open(not_zip.path().to_path_buf(), None).unwrap(), limits()).unwrap_err();
        assert_eq!(e.code, ErrorCode::ArchiveCorrupt);
    }
}
