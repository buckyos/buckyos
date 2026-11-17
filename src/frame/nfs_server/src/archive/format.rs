//! What every archive format comes down to (more-formats TODO T0, E1): the
//! format sniffed from the outer's first bytes, one member list, where each
//! member's bytes are (a range of the data file, a ZIP record, a 7z block),
//! and the one decode path reads, copies, materialization and password
//! checks share.

use super::cache::Index;
use super::names::RawName;
use super::source::*;
use super::{sevenz, zip};
use crate::error::*;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    SevenZ,
    /// One compressed file (`.gz`, `.xz`, …) that is not a tar: a one-member archive.
    Compressed,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Zip => "zip",
            Kind::Tar => "tar",
            Kind::SevenZ => "7z",
            Kind::Compressed => "compressed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Gzip,
    Xz,
    Bzip2,
    Zstd,
}

impl Compression {
    pub const ALL: [Compression; 4] = [Compression::Gzip, Compression::Xz, Compression::Bzip2, Compression::Zstd];

    pub fn name(&self) -> &'static str {
        match self {
            Compression::Gzip => "gzip",
            Compression::Xz => "xz",
            Compression::Bzip2 => "bzip2",
            Compression::Zstd => "zstd",
        }
    }

    /// File extensions of one file compressed this way.
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Compression::Gzip => &["gz"],
            Compression::Xz => &["xz"],
            Compression::Bzip2 => &["bz2"],
            Compression::Zstd => &["zst"],
        }
    }

    /// Extensions of a tar compressed this way.
    pub fn tar_extensions(&self) -> &'static [&'static str] {
        match self {
            Compression::Gzip => &["tar.gz", "tgz"],
            Compression::Xz => &["tar.xz", "txz"],
            Compression::Bzip2 => &["tar.bz2", "tbz2", "tbz"],
            Compression::Zstd => &["tar.zst", "tzst"],
        }
    }

    /// A decoder of every concatenated stream (pigz, pbzip2, `xz -T0`, zstd frames).
    pub fn decoder<'a, R: std::io::BufRead + Send + 'a>(&self, r: R, memlimit: u64) -> std::io::Result<Box<dyn Read + Send + 'a>> {
        Ok(match self {
            Compression::Gzip => Box::new(flate2::bufread::MultiGzDecoder::new(r)),
            Compression::Xz => {
                let stream = xz2::stream::Stream::new_stream_decoder(memlimit, xz2::stream::CONCATENATED).map_err(std::io::Error::other)?;
                Box::new(xz2::bufread::XzDecoder::new_stream(r, stream))
            }
            Compression::Bzip2 => Box::new(bzip2::bufread::MultiBzDecoder::new(r)),
            Compression::Zstd => {
                let mut d = zstd::stream::read::Decoder::with_buffer(r)?;
                d.window_log_max(memlimit.max(1 << 20).ilog2().min(31))?;
                Box::new(d)
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct Format {
    pub kind: Kind,
    pub compression: Option<Compression>,
    pub solid: bool,
    pub header_encrypted: bool,
}

impl Format {
    pub fn of(kind: Kind) -> Format {
        Format { kind, compression: None, solid: false, header_encrypted: false }
    }
}

/// What the first bytes of an outer say it is (E1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Zip,
    SevenZ,
    Compressed(Compression),
    Tar,
    /// A format this server knows but does not open (RAR).
    Refused(&'static str),
    /// Nothing recognized at the start; may still be a ZIP with data before it.
    Unknown,
}

pub fn sniff(head: &[u8]) -> Sniffed {
    let starts = |p: &[u8]| head.starts_with(p);
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") {
        Sniffed::Zip
    } else if starts(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        Sniffed::SevenZ
    } else if starts(&[0x1F, 0x8B, 0x08]) {
        Sniffed::Compressed(Compression::Gzip)
    } else if starts(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]) {
        Sniffed::Compressed(Compression::Xz)
    } else if head.len() >= 4 && starts(b"BZh") && (b'1'..=b'9').contains(&head[3]) {
        Sniffed::Compressed(Compression::Bzip2)
    } else if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Sniffed::Compressed(Compression::Zstd)
    } else if super::tar::is_header(head) {
        Sniffed::Tar
    } else if starts(b"Rar!\x1A\x07") {
        Sniffed::Refused("rar")
    } else {
        Sniffed::Unknown
    }
}

/// One entry of an archive, whatever its format.
#[derive(Debug, Clone)]
pub struct Member {
    pub name: RawName,
    pub size: u64,
    pub compressed: Option<u64>,
    /// The member's part of its etag (CRC, else its offset or number).
    pub tag: String,
    pub mtime: i64,
    pub encrypted: bool,
    /// A kind that is never addressable (links, devices, sparse files, anti-items).
    pub hidden: bool,
    /// False when the format cannot read this record on its own (shadowed ZIP duplicates).
    pub readable: bool,
    pub method: String,
    pub loc: Loc,
}

#[derive(Debug, Clone)]
pub enum Loc {
    /// A central directory record (`index` in the `zip` crate).
    Zip { method: u16, strong: bool, header_offset: u64, index: Option<usize> },
    /// Plain bytes at `start` of the index's data file.
    Range { start: u64 },
    /// File `file` of 7z block `block`; `solid` when the block holds other files too.
    SevenZ { file: usize, block: usize, solid: bool, unsupported: Option<String> },
}

impl Member {
    pub fn supported(&self) -> NfsResult<()> {
        match &self.loc {
            Loc::Zip { method, strong, .. } => zip::supported(*method, *strong),
            Loc::SevenZ { unsupported: Some(m), .. } => Err(unsupported("method", format!("compression method {m} is not supported"))),
            _ => Ok(()),
        }
    }

    /// Readable as a byte range of the data file.
    pub fn direct(&self) -> bool {
        match &self.loc {
            Loc::Zip { method, .. } => *method == 0 && !self.encrypted,
            Loc::Range { .. } => true,
            Loc::SevenZ { .. } => false,
        }
    }

    /// `(block, file)` of a member that only a pass over its whole block decodes cheaply.
    pub fn solid(&self) -> Option<(usize, usize)> {
        match &self.loc {
            Loc::SevenZ { block, file, solid: true, .. } => Some((*block, *file)),
            _ => None,
        }
    }
}

/// What an index reads members with.
#[derive(Debug)]
pub enum Body {
    Zip(::zip::ZipArchive<PosReader>),
    /// Every member is a `Loc::Range` of the data file.
    Ranges,
    SevenZ(Arc<sevenz_rust2::Archive>),
    /// A 7z whose header is encrypted: nothing is known until a password opens it.
    Locked,
}

/// Where a direct member's bytes start in the index's data file.
pub fn data_start(index: &Index, m: &Member) -> NfsResult<u64> {
    match &m.loc {
        Loc::Zip { header_offset, .. } => zip::data_start(&index.data, *header_offset, m.compressed.unwrap_or(0)),
        Loc::Range { start } => {
            if start.checked_add(m.size).is_none_or(|end| end > index.data.len) {
                return Err(corrupt("member data runs past the archive"));
            }
            Ok(*start)
        }
        Loc::SevenZ { .. } => Err(internal("7z members are not byte ranges")),
    }
}

/// Decodes member `m` into `sink` (blocking): `limit` stops after that many
/// bytes, `Ok(false)` means cancelled or the sink stopped.
pub fn decode(index: &Index, m: &Member, password: Option<&str>, limit: Option<u64>, cancel: &AtomicBool, sink: &mut dyn FnMut(&[u8]) -> std::io::Result<bool>) -> NfsResult<bool> {
    m.supported()?;
    match &m.loc {
        Loc::Zip { .. } => {
            let mut z = match &index.body {
                Body::Zip(z) => z.clone(),
                _ => return Err(internal("not a ZIP index")),
            };
            let reader = zip::open(&mut z, m, password.map(str::as_bytes))?;
            pump(reader, m.size, limit, cancel, sink)
        }
        Loc::Range { .. } => {
            let start = data_start(index, m)?;
            let mut r = index.data.reader();
            r.seek(SeekFrom::Start(start)).map_err(io_error)?;
            pump(r.take(m.size), m.size, limit, cancel, sink)
        }
        Loc::SevenZ { .. } => sevenz::decode(index, m, password, limit, cancel, sink),
    }
}

/// Whether `password` passes the format's cheap check on `m` (blocking).
pub fn quick_check(index: &Index, m: &Member, password: &str) -> bool {
    match &m.loc {
        Loc::Zip { .. } => match &index.body {
            Body::Zip(z) => zip::open(&mut z.clone(), m, Some(password.as_bytes())).is_ok(),
            _ => false,
        },
        Loc::Range { .. } => true,
        Loc::SevenZ { .. } => sevenz::quick_check(index, m, password),
    }
}
