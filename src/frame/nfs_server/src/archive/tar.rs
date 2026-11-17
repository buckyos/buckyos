//! tar and compressed streams (more-formats TODO E2–E4, Q3): a plain tar is
//! indexed by seeking from header to header and its files are byte ranges of
//! the outer; a compressed outer is decompressed once into the cache (one
//! pass that indexes as it writes), after which it is a plain tar, or a
//! one-member archive when the stream is not a tar.

use super::cache::{create_private_sync, ArchiveMgr};
use super::format::{Compression, Kind, Loc, Member};
use super::names::RawName;
use super::source::*;
use crate::error::*;
use serde_json::json;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const BLOCK: usize = 512;
const RESERVE_STEP: u64 = 64 * 1024 * 1024;

/// A tar header block: the POSIX magic, or a v7 header whose checksum adds up.
pub fn is_header(b: &[u8]) -> bool {
    if b.len() < BLOCK {
        return false;
    }
    if &b[257..262] == b"ustar" {
        return true;
    }
    if b[..BLOCK].iter().all(|&c| c == 0) {
        return false;
    }
    let field = &b[148..156];
    let digits: String = field.iter().take_while(|&&c| c != 0 && c != b' ').map(|&c| c as char).collect();
    let Ok(want) = u32::from_str_radix(digits.trim(), 8) else { return false };
    let sum: u32 = b[..BLOCK].iter().enumerate().map(|(i, &c)| if (148..156).contains(&i) { b' ' as u32 } else { c as u32 }).sum();
    sum == want
}

/// One tar entry as a member: `None` for the root `./` and for metadata records.
fn member<R: Read>(e: &mut tar::Entry<'_, R>) -> io::Result<Option<Member>> {
    let et = e.header().entry_type();
    if et.is_pax_global_extensions() || et.is_pax_local_extensions() || et.is_gnu_longname() || et.is_gnu_longlink() {
        return Ok(None);
    }
    let mut raw = e.path_bytes().into_owned();
    let (pax_path, pax_mtime) = match e.pax_extensions()? {
        Some(exts) => {
            let mut path = None;
            let mut mtime = None;
            for x in exts.flatten() {
                match x.key_bytes() {
                    b"path" => path = Some(x.value_bytes().to_vec()),
                    b"mtime" => mtime = x.value().ok().and_then(|v| v.split('.').next()?.parse::<i64>().ok()),
                    _ => {}
                }
            }
            (path, mtime)
        }
        None => (None, None),
    };
    while raw.starts_with(b"./") {
        raw.drain(..2);
    }
    if raw.is_empty() || raw == b"." {
        return Ok(None);
    }
    let dir = et.is_dir();
    let file = et.is_file() || et.is_contiguous();
    if dir && !raw.ends_with(b"/") {
        raw.push(b'/');
    }
    let unicode = pax_path.and_then(|_| String::from_utf8(raw.clone()).ok());
    let start = e.raw_file_position();
    let size = if file { e.size() } else { 0 };
    Ok(Some(Member {
        name: RawName { raw, unicode },
        size,
        compressed: None,
        tag: format!("{start:x}"),
        mtime: pax_mtime.unwrap_or_else(|| e.header().mtime().unwrap_or(0) as i64),
        encrypted: false,
        hidden: !(dir || file),
        readable: true,
        method: "stored".into(),
        loc: Loc::Range { start },
    }))
}

fn entries_error(e: io::Error) -> NfsError {
    if is_stale(&e) {
        return stale("the archive changed; open it again");
    }
    if let Some(inner) = e.get_ref().and_then(|i| i.downcast_ref::<Stop>()) {
        return inner.0.clone();
    }
    corrupt(format!("not a readable tar: {e}"))
}

/// Indexes a tar that sits uncompressed in `data`, skipping file contents.
pub fn parse_plain(data: &Arc<Source>, limits: Limits) -> NfsResult<Vec<Member>> {
    let mut archive = tar::Archive::new(data.reader());
    let mut out = Vec::new();
    for entry in archive.entries_with_seek().map_err(entries_error)? {
        let mut entry = entry.map_err(entries_error)?;
        if let Some(m) = member(&mut entry).map_err(entries_error)? {
            out.push(m);
            if out.len() as u64 > limits.max_entries {
                return Err(quota("entry count", limits.max_entries));
            }
        }
    }
    Ok(out)
}

/// An error the tee hands through the tar reader.
#[derive(Debug)]
struct Stop(NfsError);

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl std::error::Error for Stop {}

/// Copies what is read into the cache file, within the cache budget.
struct Tee<'a, R> {
    r: R,
    w: BufWriter<std::fs::File>,
    written: u64,
    reserved: u64,
    mgr: &'a ArchiveMgr,
    limit: u64,
    cancel: &'a AtomicBool,
}

impl<R: Read> Read for Tee<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other(Stop(internal("cancelled"))));
        }
        let n = self.r.read(buf)?;
        let total = self.written + n as u64;
        if total > self.limit {
            return Err(io::Error::other(Stop(quota("decompressed size", self.limit))));
        }
        if total > self.reserved {
            let want = (total + RESERVE_STEP).min(self.limit);
            self.mgr.reserve(want).map_err(|e| io::Error::other(Stop(e)))?;
            self.reserved = want;
        }
        self.w.write_all(&buf[..n])?;
        self.written = total;
        Ok(n)
    }
}

/// The file name a gzip header carries (FNAME), if any.
fn gzip_name(head: &[u8]) -> Option<Vec<u8>> {
    if head.len() < 10 || head[3] & 0x08 == 0 {
        return None;
    }
    let mut at = 10;
    if head[3] & 0x04 != 0 {
        let len = u16::from_le_bytes([*head.get(10)?, *head.get(11)?]) as usize;
        at = 12 + len;
    }
    let rest = head.get(at..)?;
    let end = rest.iter().position(|&b| b == 0)?;
    let name = &rest[..end];
    let base = name.rsplit(|&b| b == b'/' || b == b'\\').next()?;
    (!base.is_empty()).then(|| base.to_vec())
}

/// The name of the one member of a compressed file: the gzip FNAME, else the
/// outer name without its compression extension.
fn single_name(outer: &str, comp: Compression, head: &[u8]) -> RawName {
    if comp == Compression::Gzip {
        if let Some(raw) = gzip_name(head) {
            let unicode = String::from_utf8(raw.clone()).ok();
            return RawName { raw, unicode };
        }
    }
    let lower = outer.to_ascii_lowercase();
    let stem = comp
        .extensions()
        .iter()
        .find_map(|ext| lower.strip_suffix(&format!(".{ext}")).map(|s| &outer[..s.len()]))
        .filter(|s| !s.is_empty())
        .unwrap_or(outer);
    let stem = if stem == outer { format!("{outer}.out") } else { stem.to_string() };
    RawName { raw: stem.clone().into_bytes(), unicode: Some(stem) }
}

/// What a compressed outer turned into on disk.
pub struct Unpacked {
    pub kind: Kind,
    pub members: Vec<Member>,
    pub size: u64,
}

/// Decompresses `src` into `dest` (via `tmp`), indexing the tar inside as it
/// goes; the file appears at `dest` only when complete.
#[allow(clippy::too_many_arguments)]
pub fn unpack(src: &Arc<Source>, comp: Compression, outer_name: &str, dest: &Path, tmp: &Path, mgr: &ArchiveMgr, limits: Limits, cancel: &AtomicBool) -> NfsResult<Unpacked> {
    let head = read_exact_at(&mut src.reader(), 0, (src.len as usize).min(64 * 1024)).map_err(io_error)?;
    let decoder = comp.decoder(BufReader::with_capacity(1 << 20, src.reader()), limits.max_decoder_memory).map_err(io_error)?;
    let file = create_private_sync(tmp)?;
    let result = (|| -> NfsResult<Unpacked> {
        let mut tee = Tee { r: decoder, w: BufWriter::with_capacity(1 << 20, file), written: 0, reserved: 0, mgr, limit: limits.max_materialize_bytes, cancel };
        let mut first = Vec::with_capacity(BLOCK);
        (&mut tee).take(BLOCK as u64).read_to_end(&mut first).map_err(entries_error)?;
        let (kind, members) = if is_header(&first) {
            let mut members = Vec::new();
            {
                let mut archive = tar::Archive::new(io::Cursor::new(first).chain(&mut tee));
                for entry in archive.entries().map_err(entries_error)? {
                    let mut entry = entry.map_err(entries_error)?;
                    if let Some(m) = member(&mut entry).map_err(entries_error)? {
                        members.push(m);
                        if members.len() as u64 > limits.max_entries {
                            return Err(quota("entry count", limits.max_entries));
                        }
                    }
                }
            }
            (Kind::Tar, members)
        } else {
            (Kind::Compressed, Vec::new())
        };
        io::copy(&mut tee, &mut io::sink()).map_err(entries_error)?;
        let size = tee.written;
        let file = tee.w.into_inner().map_err(|e| NfsError::from(e.into_error()))?;
        file.sync_all()?;
        let members = match kind {
            Kind::Tar => members,
            _ => vec![Member {
                name: single_name(outer_name, comp, &head),
                size,
                compressed: Some(src.len),
                tag: "0".into(),
                mtime: if comp == Compression::Gzip && head.len() >= 8 { u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as i64 } else { 0 },
                encrypted: false,
                hidden: false,
                readable: true,
                method: comp.name().into(),
                loc: Loc::Range { start: 0 },
            }],
        };
        Ok(Unpacked { kind, members, size })
    })();
    match result {
        Ok(u) => {
            std::fs::rename(tmp, dest)?;
            let side = json!({"kind": u.kind.name(), "name": u.members.first().filter(|_| u.kind == Kind::Compressed).map(|m| String::from_utf8_lossy(&m.name.raw).into_owned())});
            std::fs::write(dest.with_extension("json"), side.to_string())?;
            mgr.record(dest, u.size);
            Ok(u)
        }
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(e)
        }
    }
}

/// An outer decompressed earlier: its index again from the file left in the cache.
pub fn reopen(data: &Arc<Source>, outer_name: &str, comp: Compression, limits: Limits) -> NfsResult<Unpacked> {
    let side: serde_json::Value = std::fs::read(data.path().with_extension("json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or_else(|| corrupt("cache entry without its record"))?;
    if side["kind"] == "tar" {
        return Ok(Unpacked { kind: Kind::Tar, members: parse_plain(data, limits)?, size: data.len });
    }
    let name = side["name"].as_str().map(|n| RawName { raw: n.as_bytes().to_vec(), unicode: Some(n.to_string()) }).unwrap_or_else(|| single_name(outer_name, comp, &[]));
    Ok(Unpacked {
        kind: Kind::Compressed,
        members: vec![Member { name, size: data.len, compressed: None, tag: "0".into(), mtime: 0, encrypted: false, hidden: false, readable: true, method: comp.name().into(), loc: Loc::Range { start: 0 } }],
        size: data.len,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A tar of `(name, data)`; names ending in `/` are directories, `->` makes a symlink.
    pub fn build(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_mtime(1_700_000_000);
            h.set_mode(0o644);
            if let Some((link, target)) = name.split_once("->") {
                h.set_entry_type(tar::EntryType::Symlink);
                h.set_size(0);
                b.append_link(&mut h, link, target).unwrap();
            } else if name.ends_with('/') {
                h.set_entry_type(tar::EntryType::Directory);
                h.set_size(0);
                b.append_data(&mut h, name, io::empty()).unwrap();
            } else {
                h.set_entry_type(tar::EntryType::Regular);
                h.set_size(data.len() as u64);
                b.append_data(&mut h, name, *data).unwrap();
            }
        }
        b.into_inner().unwrap()
    }

    #[test]
    fn headers_are_recognized() {
        let t = build(&[("a.txt", b"hello")]);
        assert!(is_header(&t));
        let mut v7 = t[..512].to_vec();
        v7[257..265].fill(0);
        let sum: u32 = v7.iter().enumerate().map(|(i, &c)| if (148..156).contains(&i) { 32 } else { c as u32 }).sum();
        v7[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        assert!(is_header(&v7));
        assert!(!is_header(&[0u8; 512]));
        assert!(!is_header(b"just some text"));
    }

    fn mgr(dir: &Path) -> ArchiveMgr {
        ArchiveMgr::new(&dir.join("data"), Default::default()).unwrap()
    }

    fn limits() -> Limits {
        Limits { max_central_directory_bytes: 1 << 20, max_entries: 1000, max_materialize_bytes: 1 << 30, max_decoder_memory: 256 << 20 }
    }

    #[test]
    fn plain_tars_index_without_reading_contents() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.tar");
        std::fs::write(&p, build(&[("./", b""), ("./d/", b""), ("./d/a.txt", b"hello"), ("./l->d/a.txt", b""), ("w\\x.txt", b"bs")])).unwrap();
        let src = Source::open(p, None).unwrap();
        let members = parse_plain(&src, limits()).unwrap();
        let names: Vec<&[u8]> = members.iter().map(|m| m.name.raw.as_slice()).collect();
        assert_eq!(names, vec![&b"d/"[..], b"d/a.txt", b"l", b"w\\x.txt"]);
        assert!(members[2].hidden);
        let a = &members[1];
        let Loc::Range { start } = a.loc else { panic!() };
        assert_eq!(read_exact_at(&mut src.reader(), start, 5).unwrap(), b"hello");
        let naming = super::super::names::Naming::new(members.iter().map(|m| m.name.clone()).collect(), members.iter().map(|m| (m.hidden, true)).collect(), false);
        let tree = naming.trees_for(None).remove(0);
        assert!(tree.by_path("d/a.txt").is_some());
        assert!(tree.by_path("w/x.txt").is_none(), "a backslash in a tar name is not a separator");
        assert_eq!(tree.hidden, 2, "the symlink and the name with a backslash");
        assert_eq!(parse_plain(&src, Limits { max_entries: 2, ..limits() }).unwrap_err().code, ErrorCode::QuotaExceeded);
    }

    #[test]
    fn unpacking_is_cancellable_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = mgr(dir.path());
        let p = dir.path().join("t.tar.gz");
        let tar = build(&[("a.txt", &vec![7u8; 200_000])]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        std::fs::write(&p, gz.finish().unwrap()).unwrap();
        let src = Source::open(p, None).unwrap();
        let (dest, tmp) = (mgr.unpacked_dir().join("x"), mgr.tmp_path(&mgr.unpacked_dir()));
        let e = unpack(&src, Compression::Gzip, "t.tar.gz", &dest, &tmp, &mgr, limits(), &AtomicBool::new(true)).err().unwrap();
        assert_eq!(e.message, "cancelled");
        assert!(!tmp.exists() && !dest.exists());
        let u = unpack(&src, Compression::Gzip, "t.tar.gz", &dest, &tmp, &mgr, limits(), &AtomicBool::new(false)).unwrap();
        assert_eq!((u.kind, u.members.len(), u.size), (Kind::Tar, 1, tar.len() as u64));
        let again = reopen(&Source::open(dest, None).unwrap(), "t.tar.gz", Compression::Gzip, limits()).unwrap();
        assert_eq!(again.members[0].size, 200_000);
    }

    #[test]
    fn concatenated_gzip_members_decode_as_one_stream() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = mgr(dir.path());
        let mut bytes = Vec::new();
        for part in [&b"hello "[..], b"world"] {
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(part).unwrap();
            bytes.extend(gz.finish().unwrap());
        }
        let p = dir.path().join("greeting.txt.gz");
        std::fs::write(&p, bytes).unwrap();
        let src = Source::open(p, None).unwrap();
        let dest = mgr.unpacked_dir().join("g");
        let u = unpack(&src, Compression::Gzip, "greeting.txt.gz", &dest, &mgr.tmp_path(&mgr.unpacked_dir()), &mgr, limits(), &AtomicBool::new(false)).unwrap();
        assert_eq!((u.kind, u.members[0].name.raw.as_slice(), u.size), (Kind::Compressed, &b"greeting.txt"[..], 11));
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
    }

    #[test]
    fn gzip_names() {
        let mut h = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
        h.extend_from_slice(b"dir/report.csv\0");
        assert_eq!(gzip_name(&h).unwrap(), b"report.csv");
        assert!(gzip_name(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3]).is_none());
        let n = single_name("log.TXT.GZ", Compression::Gzip, &[]);
        assert_eq!(n.raw, b"log.TXT");
        let n = single_name("weird", Compression::Zstd, &[]);
        assert_eq!(n.raw, b"weird.out");
    }
}
