//! 7z provider (more-formats TODO E6–E11, `sevenz-rust2`): the header read
//! into members (UTF-16 names, one entry per file), Copy blocks served as
//! byte ranges, every other member decoded through its block; a solid block
//! is only decoded from its start (the caller extracts it whole, E8).

use super::cache::Index;
use super::format::{Body, Loc, Member};
use super::names::RawName;
use super::source::*;
use crate::error::*;
use serde_json::json;
use sevenz_rust2::{Archive, BlockDecoder, EncoderMethod, Password};
use std::io::Read;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const SIGNATURE_HEADER_SIZE: u64 = 32;

/// Coders this server decodes (E10), with their names.
const CODERS: &[(&[u8], &str)] = &[
    (EncoderMethod::ID_COPY, "copy"),
    (EncoderMethod::ID_LZMA, "lzma"),
    (EncoderMethod::ID_LZMA2, "lzma2"),
    (EncoderMethod::ID_PPMD, "ppmd"),
    (EncoderMethod::ID_BZIP2, "bzip2"),
    (EncoderMethod::ID_DEFLATE, "deflate"),
    (EncoderMethod::ID_BCJ_X86, "bcj"),
    (EncoderMethod::ID_BCJ2, "bcj2"),
    (EncoderMethod::ID_BCJ_ARM, "arm"),
    (EncoderMethod::ID_BCJ_ARM64, "arm64"),
    (EncoderMethod::ID_BCJ_ARM_THUMB, "armt"),
    (EncoderMethod::ID_BCJ_PPC, "ppc"),
    (EncoderMethod::ID_BCJ_IA64, "ia64"),
    (EncoderMethod::ID_BCJ_SPARC, "sparc"),
    (EncoderMethod::ID_BCJ_RISCV, "riscv"),
    (EncoderMethod::ID_DELTA, "delta"),
    (EncoderMethod::ID_AES256_SHA256, "aes256"),
];
/// What `hello.archive` advertises: codecs and filters decoded from samples
/// (`every_writable_coder_decodes`); BCJ2 is decoded too, but nothing here
/// writes it to check.
pub const METHODS: &[&str] = &["copy", "lzma", "lzma2", "ppmd", "bzip2", "deflate"];
pub const FILTERS: &[&str] = &["bcj", "arm", "arm64", "armt", "ppc", "ia64", "sparc", "riscv", "delta"];
pub const ENCRYPTION: &[&str] = &["aes256"];

fn coder_name(id: &[u8]) -> Option<&'static str> {
    CODERS.iter().find(|(c, _)| *c == id).map(|(_, n)| *n)
}

fn mem_kb(limits: &Limits) -> usize {
    (limits.max_decoder_memory / 1024) as usize
}

pub fn error(e: sevenz_rust2::Error) -> NfsError {
    use sevenz_rust2::Error as E;
    match e {
        E::PasswordRequired => NfsError::new(ErrorCode::PasswordRequired, "this archive is encrypted"),
        E::MaybeBadPassword(e) if is_stale(&e) => io_error(e),
        E::MaybeBadPassword(e) => integrity(format!("the member does not decode: {e}")),
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch => integrity("checksum mismatch"),
        E::UnsupportedCompressionMethod(m) => unsupported("method", format!("compression method {m} is not supported")),
        E::Unsupported(m) => unsupported("method", m.to_string()),
        E::ExternalUnsupported => unsupported("method", "this 7z feature is not supported"),
        E::MaxMemLimited { max_kb, .. } => quota("decoder memory", max_kb as u64 * 1024),
        E::Io(e, _) | E::FileOpen(e, _) => io_error(e),
        E::BadSignature(_) => corrupt("not a 7z archive"),
        other => corrupt(format!("7z: {other}")),
    }
}

fn unix_time(t: sevenz_rust2::NtTime) -> i64 {
    let st: std::time::SystemTime = t.into();
    match st.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// The members of a parsed header.
fn members(a: &Archive) -> Vec<Member> {
    // Without BCJ2 (the only coder with several inputs) every block has one pack stream, in block order.
    let one_pack_each = !a.blocks.iter().any(|b| b.coders.iter().any(|c| c.encoder_method_id() == EncoderMethod::ID_BCJ2));
    let mut pack_starts = Vec::with_capacity(a.blocks.len());
    let mut at = SIGNATURE_HEADER_SIZE + a.pack_pos();
    for size in a.pack_sizes() {
        pack_starts.push(at);
        at += size;
    }
    let mut in_block = vec![0usize; a.blocks.len()];
    for (i, f) in a.files.iter().enumerate() {
        if let (Some(b), true) = (a.stream_map.file_block_index[i], f.has_stream) {
            in_block[b] += 1;
        }
    }
    let mut block_offset = vec![0u64; a.blocks.len()];
    let mut out = Vec::with_capacity(a.files.len());
    for (i, f) in a.files.iter().enumerate() {
        let mut name = f.name.clone();
        if f.is_directory && !name.ends_with('/') && !name.ends_with('\\') {
            name.push('/');
        }
        let attrs = f.windows_attributes;
        let unix_link = f.has_windows_attributes && attrs & 0x8000 != 0 && (attrs >> 16) & 0o170000 == 0o120000;
        let reparse = f.has_windows_attributes && attrs & 0x400 != 0;
        let block = a.stream_map.file_block_index[i].filter(|_| f.has_stream && f.size > 0);
        let (loc, method, compressed) = match block {
            None => (Loc::Range { start: 0 }, "stored".to_string(), None),
            Some(b) => {
                let coders = &a.blocks[b].coders;
                let unsupported = coders.iter().find(|c| coder_name(c.encoder_method_id()).is_none()).map(|c| hex::encode(c.encoder_method_id()));
                let method = coders.iter().filter_map(|c| coder_name(c.encoder_method_id())).find(|n| METHODS.contains(n)).unwrap_or("copy").to_string();
                let copy = coders.len() == 1 && coders[0].encoder_method_id() == EncoderMethod::ID_COPY;
                let loc = if copy && one_pack_each && b < pack_starts.len() {
                    let start = pack_starts[b] + block_offset[b];
                    block_offset[b] += f.size;
                    Loc::Range { start }
                } else {
                    Loc::SevenZ { file: i, block: b, solid: in_block[b] > 1, unsupported }
                };
                let compressed = (in_block[b] == 1 && one_pack_each).then(|| a.pack_sizes().get(b).copied()).flatten();
                (loc, method, compressed)
            }
        };
        out.push(Member {
            name: RawName { raw: name.as_bytes().to_vec(), unicode: Some(name) },
            size: if block.is_some() { f.size } else { 0 },
            compressed,
            tag: if f.has_crc { format!("{:08x}", f.crc) } else { format!("i{i}") },
            mtime: if f.has_last_modified_date { unix_time(f.last_modified_date) } else { 0 },
            encrypted: block.is_some() && a.is_file_encrypted(i),
            hidden: f.is_anti_item || unix_link || reparse,
            readable: true,
            method,
            loc,
        });
    }
    out
}

/// The parsed header of a 7z, or `Body::Locked` when it is encrypted.
pub fn parse(src: &Arc<Source>, limits: Limits) -> NfsResult<(Body, Vec<Member>, bool)> {
    match Archive::read_with_memory_limit(&mut src.reader(), &Password::empty(), mem_kb(&limits)) {
        Ok(a) => {
            if a.files.len() as u64 > limits.max_entries {
                return Err(quota("entry count", limits.max_entries));
            }
            let members = members(&a);
            let solid = a.is_solid;
            Ok((Body::SevenZ(Arc::new(a)), members, solid))
        }
        Err(sevenz_rust2::Error::PasswordRequired) => Ok((Body::Locked, Vec::new(), false)),
        Err(e) => Err(error(e)),
    }
}

/// The header of an encrypted-header 7z opened with `password` (E7): a full
/// check, since the header carries a CRC.
pub fn unlock(src: &Arc<Source>, password: &str, limits: Limits) -> NfsResult<(Body, Vec<Member>, bool)> {
    match Archive::read_with_memory_limit(&mut src.reader(), &Password::new(password), mem_kb(&limits)) {
        Ok(a) => {
            if a.files.len() as u64 > limits.max_entries {
                return Err(quota("entry count", limits.max_entries));
            }
            let members = members(&a);
            let solid = a.is_solid;
            Ok((Body::SevenZ(Arc::new(a)), members, solid))
        }
        Err(e) => match error(e) {
            e if matches!(e.code, ErrorCode::Stale | ErrorCode::QuotaExceeded | ErrorCode::Unsupported) => Err(e),
            _ => Err(NfsError::new(ErrorCode::PasswordInvalid, "the password does not open this archive")),
        },
    }
}

fn archive(index: &Index) -> NfsResult<Arc<Archive>> {
    match &index.body {
        Body::SevenZ(a) => Ok(a.clone()),
        Body::Locked => Err(NfsError::new(ErrorCode::PasswordRequired, "this archive is encrypted; unlock it first")),
        _ => Err(internal("not a 7z index")),
    }
}

/// Decodes block `block` from its start, handing each file of it to `each`
/// in order (`each` returns false to stop). Entries `each` leaves unread are
/// drained and checked by the decoder.
pub fn for_block(index: &Index, block: usize, password: Option<&str>, each: &mut dyn FnMut(usize, &mut dyn Read) -> NfsResult<bool>) -> NfsResult<()> {
    let a = archive(index)?;
    let pw = password.map(Password::new).unwrap_or_else(Password::empty);
    let mut reader = index.src.reader();
    let mut file = a.stream_map.block_first_file_index[block];
    let mut failed: Option<NfsError> = None;
    let decoder = BlockDecoder::new_with_memory_limit(1, mem_kb(&index.limits), block, &a, &pw, &mut reader);
    let result = decoder.for_each_entries(&mut |_, r| {
        let i = file;
        file += 1;
        match each(i, r) {
            Ok(go) => Ok(go),
            Err(e) => {
                failed = Some(e);
                Err(sevenz_rust2::Error::Other("stopped".into()))
            }
        }
    });
    if let Some(e) = failed {
        return Err(e);
    }
    result.map(|_| ()).map_err(error)
}

/// Decodes one member (its block from the start up to it).
pub fn decode(index: &Index, m: &Member, password: Option<&str>, limit: Option<u64>, cancel: &AtomicBool, sink: &mut dyn FnMut(&[u8]) -> std::io::Result<bool>) -> NfsResult<bool> {
    let Loc::SevenZ { file, block, .. } = m.loc else { return Err(internal("not a 7z member")) };
    if m.encrypted && password.is_none() {
        return Err(NfsError::new(ErrorCode::PasswordRequired, "this member is encrypted"));
    }
    let mut out: Option<NfsResult<bool>> = None;
    for_block(index, block, password, &mut |i, r| {
        if i != file {
            return Ok(true);
        }
        out = Some(pump(r, m.size, limit, cancel, sink));
        Ok(false)
    })?;
    out.unwrap_or_else(|| Err(corrupt("the member is missing from its block")))
}

/// The quick check (E9): the first bytes of `m`'s block decode with `password`.
pub fn quick_check(index: &Index, m: &Member, password: &str) -> bool {
    let Loc::SevenZ { block, .. } = m.loc else { return true };
    let mut budget = 64 * 1024usize;
    let mut buf = vec![0u8; 8192];
    for_block(index, block, Some(password), &mut |_, r| {
        while budget > 0 {
            let n = r.read(&mut buf[..budget.min(8192)]).map_err(decode_error)?;
            if n == 0 {
                break;
            }
            budget -= n;
        }
        Ok(budget > 0)
    })
    .is_ok()
}

pub fn summary_encoding() -> serde_json::Value {
    json!({"encoding": "UTF-16LE", "by": "format"})
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration};

    /// A 7z of `(name, data)` (names ending in `/` are directories).
    pub fn build(path: &std::path::Path, entries: &[(&str, &[u8])], solid: bool, password: Option<&str>, encrypt_header: bool, copy: bool) {
        let mut w = ArchiveWriter::create(path).unwrap();
        let mut methods: Vec<EncoderConfiguration> = Vec::new();
        if let Some(p) = password {
            methods.push(AesEncoderOptions::new(Password::new(p)).into());
        }
        methods.push(if copy { EncoderMethod::COPY.into() } else { Lzma2Options::from_level(1).into() });
        w.set_content_methods(methods);
        w.set_encrypt_header(encrypt_header);
        let dirs: Vec<&str> = entries.iter().filter(|(n, _)| n.ends_with('/')).map(|(n, _)| n.trim_end_matches('/')).collect();
        for d in dirs {
            w.push_archive_entry::<&[u8]>(ArchiveEntry::new_directory(d), None).unwrap();
        }
        let files: Vec<&(&str, &[u8])> = entries.iter().filter(|(n, _)| !n.ends_with('/')).collect();
        if solid {
            let list = files.iter().map(|(n, d)| {
                let mut e = ArchiveEntry::new_file(n);
                e.size = d.len() as u64;
                e
            });
            let readers = files.iter().map(|(_, d)| (*d).into()).collect();
            w.push_archive_entries(list.collect(), readers).unwrap();
        } else {
            for (n, d) in files {
                w.push_archive_entry(ArchiveEntry::new_file(n), Some(*d)).unwrap();
            }
        }
        w.finish().unwrap();
    }

    fn index_of(dir: &std::path::Path, entries: &[(&str, &[u8])], solid: bool, password: Option<&str>, copy: bool) -> (super::super::cache::ArchiveMgr, Index) {
        let p = dir.join("t.7z");
        build(&p, entries, solid, password, false, copy);
        let mgr = super::super::cache::ArchiveMgr::new(&dir.join("data"), Default::default()).unwrap();
        let index = Index::build(super::super::cache::Build { mgr: &mgr, unpack: true, cancel: &AtomicBool::new(false) }, "k".into(), p, None, "t.7z".into(), None).unwrap();
        (mgr, index)
    }

    fn read_all(index: &Index, m: &Member, password: Option<&str>) -> NfsResult<Vec<u8>> {
        let mut out = Vec::new();
        super::super::format::decode(index, m, password, None, &AtomicBool::new(false), &mut |c| {
            out.extend_from_slice(c);
            Ok(true)
        })?;
        Ok(out)
    }

    #[test]
    fn solid_members_decode_and_a_bad_crc_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..100_000u32).map(|i| (i % 97) as u8).collect();
        let (_mgr, index) = index_of(dir.path(), &[("a.txt", b"first"), ("b.bin", &big), ("c.txt", b"third")], true, None, false);
        assert!(index.format.solid);
        let c = index.members.iter().find(|m| m.name.raw == b"c.txt").unwrap().clone();
        assert!(c.solid().is_some());
        assert_eq!(read_all(&index, &c, None).unwrap(), b"third");
        let Body::SevenZ(a) = &index.body else { panic!() };
        let mut bad = (**a).clone();
        let Loc::SevenZ { file, .. } = c.loc else { panic!() };
        bad.files[file].crc ^= 1;
        let tampered = Index { body: Body::SevenZ(Arc::new(bad)), ..super::super::cache::tests::shell(&index) };
        let e = read_all(&tampered, &c, None).unwrap_err();
        assert_eq!((e.code, e.details["reason"].as_str()), (ErrorCode::ArchiveCorrupt, Some("decrypt_or_integrity")), "{e:?}");
    }

    #[test]
    fn the_quick_check_tells_a_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let (_mgr, index) = index_of(dir.path(), &[("a.txt", b"secret one"), ("b.txt", b"secret two")], true, Some("right"), false);
        let b = index.members.iter().find(|m| m.name.raw == b"b.txt").unwrap().clone();
        assert!(b.encrypted);
        assert!(super::super::format::quick_check(&index, &b, "right"));
        assert!(!super::super::format::quick_check(&index, &b, "wrong"));
        assert_eq!(read_all(&index, &b, None).unwrap_err().code, ErrorCode::PasswordRequired);
        assert_eq!(read_all(&index, &b, Some("right")).unwrap(), b"secret two");
    }

    #[test]
    fn every_writable_coder_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..50_000u32).map(|i| ((i * 7) % 251) as u8).collect();
        let lzma2 = || -> EncoderConfiguration { Lzma2Options::from_level(1).into() };
        let cases: Vec<(&str, Vec<EncoderConfiguration>)> = vec![
            ("copy", vec![EncoderMethod::COPY.into()]),
            ("lzma", vec![EncoderMethod::LZMA.into()]),
            ("lzma2", vec![lzma2()]),
            ("ppmd", vec![EncoderMethod::PPMD.into()]),
            ("bzip2", vec![EncoderMethod::BZIP2.into()]),
            ("deflate", vec![EncoderMethod::DEFLATE.into()]),
            ("bcj", vec![EncoderMethod::BCJ_X86_FILTER.into(), lzma2()]),
            ("arm", vec![EncoderMethod::BCJ_ARM_FILTER.into(), lzma2()]),
            ("arm64", vec![EncoderMethod::BCJ_ARM64_FILTER.into(), lzma2()]),
            ("armt", vec![EncoderMethod::BCJ_ARM_THUMB_FILTER.into(), lzma2()]),
            ("ppc", vec![EncoderMethod::BCJ_PPC_FILTER.into(), lzma2()]),
            ("ia64", vec![EncoderMethod::BCJ_IA64_FILTER.into(), lzma2()]),
            ("sparc", vec![EncoderMethod::BCJ_SPARC_FILTER.into(), lzma2()]),
            ("riscv", vec![EncoderMethod::BCJ_RISCV_FILTER.into(), lzma2()]),
            ("delta", vec![EncoderMethod::DELTA_FILTER.into(), lzma2()]),
        ];
        let mgr = super::super::cache::ArchiveMgr::new(&dir.path().join("data"), Default::default()).unwrap();
        let mut seen = Vec::new();
        for (name, methods) in cases {
            let p = dir.path().join(format!("{name}.7z"));
            let mut w = ArchiveWriter::create(&p).unwrap();
            w.set_content_methods(methods);
            w.push_archive_entry(ArchiveEntry::new_file("d.bin"), Some(data.as_slice())).unwrap();
            w.finish().unwrap();
            let index = Index::build(super::super::cache::Build { mgr: &mgr, unpack: true, cancel: &AtomicBool::new(false) }, format!("k-{name}"), p, None, format!("{name}.7z"), None).unwrap();
            let m = index.members[0].clone();
            assert_eq!(read_all(&index, &m, None).unwrap(), data, "{name}");
            seen.push(name);
        }
        assert_eq!(seen.iter().filter(|n| METHODS.contains(n)).count(), METHODS.len());
        assert_eq!(seen.iter().filter(|n| FILTERS.contains(n)).count(), FILTERS.len());
    }

    #[test]
    fn copy_blocks_are_byte_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let (_mgr, index) = index_of(dir.path(), &[("a.txt", b"alpha"), ("b.txt", b"bravo!"), ("c.txt", b"charlie")], true, None, true);
        for (name, want) in [(&b"a.txt"[..], &b"alpha"[..]), (b"b.txt", b"bravo!"), (b"c.txt", b"charlie")] {
            let m = index.members.iter().find(|m| m.name.raw == name).unwrap();
            assert!(m.direct(), "{}", String::from_utf8_lossy(name));
            let start = super::super::format::data_start(&index, m).unwrap();
            assert_eq!(read_exact_at(&mut index.data.reader(), start, want.len()).unwrap(), want);
        }
    }
}
