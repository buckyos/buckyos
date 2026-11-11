//! What a source is, before (magic bytes: the planning decision) and after
//! (`ffprobe` JSON: the execution facts) a conversion starts.
//!
//! Office documents (Office import TODO §5.1): OOXML, ODF, iWork and Visio
//! are zips told apart by their entry names, the legacy binaries OLE2 compound
//! files by their stream names, RTF by `{\rtf`; the file name only settles
//! what the bytes leave open (text markup, TGA, the RAW flavours of TIFF).

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// HEIF / HEIC still image (tile grid, `irot`, alpha from the `meta` box).
    Heif,
    /// Another still image ffmpeg decodes (TIFF, PSD, BMP, …).
    Image,
    Video,
    Audio,
    /// A document LibreOffice converts; the extension it picks the import filter by (`docx`, `odp`, `rtf`, …).
    Document(&'static str),
    /// A still image ImageMagick decodes; its coder (`dng`, `xcf`, `jxl`, …).
    Raster(&'static str),
    /// Text markup pandoc reads; its reader (`epub`, `ipynb`, `rst`, …).
    Markup(&'static str),
    /// PDF: shown directly, converted only for a thumbnail.
    Pdf,
}

/// The LibreOffice application a document needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Office {
    Writer,
    Calc,
    Impress,
}

impl Office {
    pub fn name(&self) -> &'static str {
        match self {
            Office::Writer => "Writer",
            Office::Calc => "Calc",
            Office::Impress => "Impress",
        }
    }
}

pub fn office_of(ext: &str) -> Office {
    match ext {
        "xls" | "xlsx" | "ods" | "numbers" => Office::Calc,
        "ppt" | "pptx" | "odp" | "key" | "odg" | "vsdx" | "pub" => Office::Impress,
        _ => Office::Writer,
    }
}

/// Spreadsheets shown as a sheet (converted to xlsx), not as PDF pages; xlsx itself the viewer reads.
pub fn is_sheet(ext: &str) -> bool {
    matches!(ext, "xls" | "ods" | "numbers")
}

/// `(extension, pandoc reader)`: the text formats pandoc is sure to render
/// whole (TODO B0; EPUB is told by its zip). reStructuredText and Org lose
/// project roles and macros, real DocBook and Typst fail to parse, LaTeX
/// is read in part: those stay out.
pub const MARKUP: &[(&str, &str)] = &[("ipynb", "ipynb"), ("md", "gfm"), ("markdown", "gfm")];

/// RAW flavours a TIFF header alone does not tell from TIFF.
const TIFF_RAW: &[&str] = &["dng", "cr2", "nef", "arw"];

pub fn is_ole2(head: &[u8]) -> bool {
    head.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])
}

/// Byte offset of an OLE2 file's first directory sector.
pub fn ole_directory_offset(head: &[u8]) -> Option<u64> {
    let shift = u16_at(head, 30).filter(|s| (9..=16).contains(s))?;
    let sector = u32::from_le_bytes(head.get(48..52)?.try_into().ok()?);
    (sector < 0xffff_fff0).then(|| (sector as u64 + 1) << shift)
}

/// Whether IFD0 of a TIFF carries `tag`.
fn tiff_has_tag(head: &[u8], tag: u16) -> bool {
    let le = head.starts_with(b"II");
    let u16_of = |at: usize| head.get(at..at + 2).map(|v| if le { u16::from_le_bytes([v[0], v[1]]) } else { u16::from_be_bytes([v[0], v[1]]) });
    let Some(ifd) = head.get(4..8).map(|v| if le { u32::from_le_bytes([v[0], v[1], v[2], v[3]]) } else { u32::from_be_bytes([v[0], v[1], v[2], v[3]]) } as usize) else { return false };
    let count = u16_of(ifd).unwrap_or(0) as usize;
    (0..count).any(|i| u16_of(ifd + 2 + i * 12) == Some(tag))
}

fn extension(name: Option<&str>) -> Option<String> {
    name.and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase())
}

/// Text, as far as the head shows: UTF-8 (a cut last character aside) without NUL.
fn looks_like_text(head: &[u8]) -> bool {
    !head.contains(&0) && match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > head.len(),
    }
}

/// A still image for ImageMagick, by its coder.
fn raster(head: &[u8], ext: Option<&str>) -> Option<&'static str> {
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        if ascii(head, 8, 2) == b"CR" {
            return Some("cr2");
        }
        if tiff_has_tag(head, 0xc612) {
            return Some("dng");
        }
        return ext.and_then(|e| TIFF_RAW.iter().copied().find(|r| *r == e));
    }
    if head.starts_with(b"FUJIFILMCCD-RAW") {
        return Some("raf");
    }
    if head.starts_with(b"IIRO") || head.starts_with(b"IIRS") || head.starts_with(b"MMOR") {
        return Some("orf");
    }
    if head.starts_with(b"IIU\0") {
        return Some("rw2");
    }
    if head.starts_with(b"gimp xcf") {
        return Some("xcf");
    }
    if head.starts_with(&[0xff, 0x0a]) || head.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n") {
        return Some("jxl");
    }
    if head.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") {
        return Some("jp2");
    }
    if head.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        return Some("exr");
    }
    if head.starts_with(b"#?RADIANCE") || head.starts_with(b"#?RGBE") {
        return Some("hdr");
    }
    if head.starts_with(b"DDS ") {
        return Some("dds");
    }
    if ascii(head, 128, 4) == b"DICM" {
        return Some("dcm");
    }
    None
}

/// Markup for pandoc: EPUB by its zip, the text formats by name once the content reads as text.
fn markup(head: &[u8], ext: Option<&str>) -> Option<&'static str> {
    if head.starts_with(b"PK\x03\x04") {
        return matches!(zip_entries(head).first(), Some(&(b"mimetype", 0, data)) if data.starts_with(b"application/epub+zip")).then_some("epub");
    }
    let reader = MARKUP.iter().find(|(e, _)| Some(*e) == ext).map(|(_, r)| *r)?;
    looks_like_text(head).then_some(reader)
}

fn ascii(b: &[u8], at: usize, len: usize) -> &[u8] {
    b.get(at..at + len).unwrap_or_default()
}

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at + 2).map(|v| u16::from_le_bytes([v[0], v[1]]) as usize)
}

/// `(name, method, data…)` of each zip local header in `head`. Found by
/// signature, not by hopping sizes: writers that stream (LibreOffice) put the
/// sizes in a data descriptor after the data.
fn zip_entries(head: &[u8]) -> Vec<(&[u8], usize, &[u8])> {
    let mut entries = Vec::new();
    let mut at = 0;
    while let Some(i) = head.get(at..).and_then(|rest| rest.windows(4).position(|w| w == b"PK\x03\x04")) {
        let h = at + i;
        at = h + 4;
        let (Some(method), Some(name_len), Some(extra_len)) = (u16_at(head, h + 8), u16_at(head, h + 26), u16_at(head, h + 28)) else { break };
        let Some(name) = head.get(h + 30..h + 30 + name_len) else { break };
        entries.push((name, method, head.get(h + 30 + name_len + extra_len..).unwrap_or_default()));
    }
    entries
}

/// Whether an OLE2 directory entry (128 bytes on a 128-byte boundary after the
/// 512-byte header) names `stream`.
fn ole_stream(head: &[u8], stream: &str) -> bool {
    let name: Vec<u8> = stream.encode_utf16().flat_map(u16::to_le_bytes).collect();
    head.chunks_exact(128).skip(4).any(|e| e.starts_with(&name) && u16_at(e, 64) == Some(name.len() + 2))
}

/// The import extension of an Office / ODF / RTF / iWork document.
fn document(head: &[u8], ext: Option<&str>) -> Option<&'static str> {
    let hinted = |allowed: &[(&str, &'static str)]| allowed.iter().find(|(e, _)| Some(*e) == ext).map(|(_, to)| *to);
    if head.starts_with(b"{\\rtf") {
        return Some("rtf");
    }
    if head.starts_with(&[0xff, b'W', b'P', b'C']) {
        return Some("wpd");
    }
    // HWP 97 (3.0), the one LibreOffice imports; HWP 5 is an OLE2 file it does not read.
    if head.starts_with(b"HWP Document File") {
        return Some("hwp");
    }
    if is_ole2(head) {
        let streams = [("WordDocument", "doc"), ("PowerPoint Document", "ppt"), ("Workbook", "xls"), ("Book", "xls"), ("Quill", "pub")];
        const NAMES: &[(&str, &str)] = &[
            ("doc", "doc"), ("dot", "doc"), ("wps", "doc"), ("wpt", "doc"),
            ("ppt", "ppt"), ("pps", "ppt"), ("pot", "ppt"), ("dps", "ppt"), ("dpt", "ppt"),
            ("xls", "xls"), ("xlt", "xls"), ("et", "xls"), ("ett", "xls"),
            ("pub", "pub"),
        ];
        return streams.iter().find(|(s, _)| ole_stream(head, s)).map(|(_, e)| *e).or_else(|| hinted(NAMES));
    }
    if !head.starts_with(b"PK\x03\x04") {
        return None;
    }
    let entries = zip_entries(head);
    if let Some(&(b"mimetype", 0, data)) = entries.first() {
        let odf = data.strip_prefix(b"application/vnd.oasis.opendocument.").unwrap_or_default();
        let kinds: [(&[u8], &'static str); 4] = [(b"text", "odt"), (b"presentation", "odp"), (b"spreadsheet", "ods"), (b"graphics", "odg")];
        if let Some((_, e)) = kinds.iter().find(|(k, _)| odf.starts_with(k)) {
            return Some(e);
        }
        if data.starts_with(b"application/epub+zip") {
            return None;
        }
    }
    let parts: [(&[u8], &'static str); 4] = [(b"word/document", "docx"), (b"ppt/presentation", "pptx"), (b"xl/workbook", "xlsx"), (b"visio/document", "vsdx")];
    for (entry, _, _) in &entries {
        if let Some((_, e)) = parts.iter().find(|(p, _)| entry.starts_with(p) && entry.ends_with(b".xml")) {
            return Some(e);
        }
        if entry.starts_with(b"Index/") && entry.ends_with(b".iwa") {
            return hinted(&[("pages", "pages"), ("key", "key"), ("numbers", "numbers")]);
        }
    }
    hinted(&[
        ("docx", "docx"), ("docm", "docx"), ("dotx", "docx"), ("dotm", "docx"),
        ("pptx", "pptx"), ("pptm", "pptx"), ("ppsx", "pptx"), ("potx", "pptx"),
        ("xlsx", "xlsx"), ("xlsm", "xlsx"), ("xltx", "xlsx"),
        ("odt", "odt"), ("odp", "odp"), ("ods", "ods"), ("odg", "odg"), ("vsdx", "vsdx"),
        ("pages", "pages"), ("key", "key"), ("numbers", "numbers"),
    ])
}

/// Planning by magic bytes (`name` only as a hint); `None` = no converter for this content.
pub fn sniff(head: &[u8], name: Option<&str>) -> Option<Kind> {
    if head.len() < 12 {
        return None;
    }
    let ext = extension(name);
    let ext = ext.as_deref();
    if let Some(ext) = document(head, ext) {
        return Some(Kind::Document(ext));
    }
    if head.starts_with(b"%PDF-") {
        return Some(Kind::Pdf);
    }
    if let Some(coder) = raster(head, ext) {
        return Some(Kind::Raster(coder));
    }
    if let Some(reader) = markup(head, ext) {
        return Some(Kind::Markup(reader));
    }
    if ascii(head, 4, 4) == b"ftyp" {
        let brand = ascii(head, 8, 4);
        return match brand {
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"hevm" | b"hevs" | b"mif1" | b"msf1" => Some(Kind::Heif),
            b"avif" | b"avis" => Some(Kind::Image),
            b"M4A " | b"M4B " => Some(Kind::Audio),
            _ => Some(Kind::Video),
        };
    }
    // QuickTime files that start with a `wide`, `mdat` or `moov` atom.
    if matches!(ascii(head, 4, 4), b"moov" | b"mdat" | b"wide" | b"free" | b"skip") {
        return Some(Kind::Video);
    }
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some(Kind::Video);
    }
    if head.starts_with(b"RIFF") {
        return match ascii(head, 8, 4) {
            b"AVI " => Some(Kind::Video),
            b"WAVE" => Some(Kind::Audio),
            b"WEBP" => Some(Kind::Image),
            _ => None,
        };
    }
    if head.starts_with(&[0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11]) || head.starts_with(b"FLV\x01") || head.starts_with(b".RMF") {
        return Some(Kind::Video);
    }
    if head.starts_with(b"FORM") && matches!(ascii(head, 8, 4), b"AIFF" | b"AIFC") || head.starts_with(b"caff") || head.starts_with(b"MAC ") {
        return Some(Kind::Audio);
    }
    if head.starts_with(&[0x00, 0x00, 0x01, 0xba]) || head.starts_with(&[0x47]) && head.get(188) == Some(&0x47) {
        return Some(Kind::Video);
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") || head.starts_with(b"8BPS") || head.starts_with(b"BM")
        || head.starts_with(&[0x89, b'P', b'N', b'G']) || head.starts_with(&[0xff, 0xd8, 0xff]) || head.starts_with(b"GIF8")
    {
        return Some(Kind::Image);
    }
    if head.starts_with(b"ID3") || head.starts_with(b"fLaC") || head.starts_with(b"OggS") || head.starts_with(b"#!AMR")
        || (head[0] == 0xff && head[1] & 0xe0 == 0xe0)
    {
        return Some(Kind::Audio);
    }
    // TGA has no magic: the name, on content that is not text.
    (ext == Some("tga") && !looks_like_text(head)).then_some(Kind::Raster("tga"))
}

#[derive(Debug, Clone, Default)]
pub struct Stream {
    pub index: u32,
    pub codec_type: String,
    pub codec_name: String,
    pub codec_tag: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub color_transfer: String,
    pub color_primaries: String,
    pub color_space: String,
    pub default: bool,
    pub attached_pic: bool,
    /// Display-matrix rotation in degrees (ffprobe sign convention).
    pub rotation: i64,
}

impl Stream {
    pub fn hdr(&self) -> bool {
        matches!(self.color_transfer.as_str(), "smpte2084" | "arib-std-b67")
    }

    pub fn has_alpha(&self) -> bool {
        let p = self.pix_fmt.as_str();
        p.starts_with("yuva") || p.starts_with("rgba") || p.starts_with("bgra") || p.starts_with("argb") || p.starts_with("abgr")
            || p.starts_with("gbrap") || p.starts_with("ya") || p == "pal8"
    }
}

#[derive(Debug, Clone, Default)]
pub struct TileGrid {
    pub width: u32,
    pub height: u32,
    pub h_offset: u32,
    pub v_offset: u32,
    pub tiles: Vec<(u32, u32, u32)>,
    pub default: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MediaInfo {
    pub format_name: String,
    pub duration: Option<f64>,
    pub streams: Vec<Stream>,
    pub grids: Vec<TileGrid>,
}

fn u32_of(v: &Value) -> u32 {
    v.as_u64().map(|n| n as u32).or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn stream_of(s: &Value) -> Stream {
    let rotation = s["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|d| d["rotation"].as_i64().or_else(|| d["rotation"].as_f64().map(|f| f.round() as i64)))
        .or_else(|| s["tags"]["rotate"].as_str().and_then(|r| r.parse::<i64>().ok()).map(|r| -r))
        .unwrap_or(0);
    Stream {
        index: u32_of(&s["index"]),
        codec_type: str_of(&s["codec_type"]),
        codec_name: str_of(&s["codec_name"]),
        codec_tag: str_of(&s["codec_tag_string"]),
        width: u32_of(&s["width"]),
        height: u32_of(&s["height"]),
        pix_fmt: str_of(&s["pix_fmt"]),
        color_transfer: str_of(&s["color_transfer"]),
        color_primaries: str_of(&s["color_primaries"]),
        color_space: str_of(&s["color_space"]),
        default: s["disposition"]["default"].as_i64() == Some(1),
        attached_pic: s["disposition"]["attached_pic"].as_i64() == Some(1),
        rotation,
    }
}

impl MediaInfo {
    pub fn parse(json: &Value) -> MediaInfo {
        let duration = json["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()).filter(|d| d.is_finite() && *d > 0.0);
        let streams = json["streams"].as_array().map(|a| a.iter().map(stream_of).collect()).unwrap_or_default();
        let grids = json["stream_groups"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|g| g["type"].as_str() == Some("Tile Grid"))
            .filter_map(|g| {
                let c = g["components"].as_array()?.first()?;
                Some(TileGrid {
                    width: u32_of(&c["width"]),
                    height: u32_of(&c["height"]),
                    h_offset: u32_of(&c["horizontal_offset"]),
                    v_offset: u32_of(&c["vertical_offset"]),
                    tiles: c["subcomponents"]
                        .as_array()?
                        .iter()
                        .map(|t| (u32_of(&t["stream_index"]), u32_of(&t["tile_horizontal_offset"]), u32_of(&t["tile_vertical_offset"])))
                        .collect(),
                    default: g["disposition"]["default"].as_i64() == Some(1),
                })
            })
            .filter(|g| g.width > 0 && g.height > 0 && !g.tiles.is_empty())
            .collect();
        MediaInfo { format_name: str_of(&json["format"]["format_name"]), duration, streams, grids }
    }

    pub fn video(&self) -> Option<&Stream> {
        self.streams.iter().find(|s| s.codec_type == "video" && !s.attached_pic)
    }

    pub fn audio(&self) -> Option<&Stream> {
        self.streams.iter().find(|s| s.codec_type == "audio")
    }

    /// The primary image of a HEIF file: the default tile grid, else the default stream.
    pub fn primary_grid(&self) -> Option<&TileGrid> {
        self.grids.iter().find(|g| g.default).or_else(|| self.grids.first())
    }

    pub fn primary_stream(&self) -> Option<&Stream> {
        let videos = || self.streams.iter().filter(|s| s.codec_type == "video");
        videos().find(|s| s.default).or_else(|| videos().next())
    }

    /// The alpha plane of a non-grid HEIF: a gray stream the size of the primary image.
    pub fn alpha_for(&self, primary: &Stream) -> Option<&Stream> {
        self.streams.iter().find(|s| {
            s.index != primary.index && s.codec_type == "video" && s.pix_fmt.starts_with("gray") && s.width == primary.width && s.height == primary.height
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sniffing() {
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0mif1", None), Some(Kind::Heif));
        assert_eq!(sniff(b"\0\0\0\x18ftypmif1\0\0\0\0heic", None), Some(Kind::Heif));
        assert_eq!(sniff(b"\0\0\0\x14ftypqt  \0\0\0\0qt  ", None), Some(Kind::Video));
        assert_eq!(sniff(b"\0\0\0\x18ftypisom\0\0\x02\0isom", None), Some(Kind::Video));
        assert_eq!(sniff(b"\0\0\0\x18ftypM4A \0\0\0\0M4A ", None), Some(Kind::Audio));
        assert_eq!(sniff(b"II*\0\x08\0\0\0\0\0\0\0", None), Some(Kind::Image));
        assert_eq!(sniff(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3", None), Some(Kind::Pdf));
        assert_eq!(sniff(b"PK\x03\x04\x14\0\0\0\0\0\0\0", None), None);
    }

    #[test]
    fn more_media() {
        let pad = |magic: &[u8]| [magic, &[0u8; 16][..]].concat();
        assert_eq!(sniff(&pad(&[0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11]), None), Some(Kind::Video));
        assert_eq!(sniff(&pad(b"FLV\x01\x05"), None), Some(Kind::Video));
        assert_eq!(sniff(&pad(b".RMF\0\0\0\x12"), None), Some(Kind::Video));
        assert_eq!(sniff(b"FORM\0\0\x10\0AIFFCOMM", None), Some(Kind::Audio));
        assert_eq!(sniff(b"FORM\0\0\x10\0AIFCFVER", None), Some(Kind::Audio));
        assert_eq!(sniff(b"FORM\0\0\x10\0ILBMBMHD", None), None);
        assert_eq!(sniff(&pad(b"caff\0\x01\0\0desc"), None), Some(Kind::Audio));
        assert_eq!(sniff(&pad(b"MAC \x96\x0f\0\0"), None), Some(Kind::Audio));
    }

    #[test]
    fn stills_for_imagemagick() {
        let pad = |magic: &[u8]| [magic, &[0u8; 200][..]].concat();
        let mut dng = b"II*\0\x08\0\0\0\x02\0".to_vec();
        dng.extend([0x00, 0x01, 3, 0, 1, 0, 0, 0, 8, 0, 0, 0]);
        dng.extend([0x12, 0xc6, 1, 0, 4, 0, 0, 0, 1, 4, 0, 0]);
        dng.extend([0u8; 8]);
        assert_eq!(sniff(&dng, None), Some(Kind::Raster("dng")));
        let mut dng_be = b"MM\0*\0\0\0\x08\0\x01".to_vec();
        dng_be.extend([0xc6, 0x12, 0, 1, 0, 0, 0, 4, 1, 4, 0, 0]);
        dng_be.extend([0u8; 8]);
        assert_eq!(sniff(&dng_be, Some("x.tif")), Some(Kind::Raster("dng")));
        assert_eq!(sniff(b"II*\0\x10\0\0\0CR\x02\0\0\0\0\0", None), Some(Kind::Raster("cr2")));
        let tiff = b"II*\0\x08\0\0\0\0\0\0\0";
        assert_eq!(sniff(tiff, Some("scan.tiff")), Some(Kind::Image));
        assert_eq!(sniff(tiff, Some("DSC_0001.NEF")), Some(Kind::Raster("nef")));
        assert_eq!(sniff(tiff, Some("a.arw")), Some(Kind::Raster("arw")));
        assert_eq!(sniff(&pad(b"FUJIFILMCCD-RAW 0201"), None), Some(Kind::Raster("raf")));
        assert_eq!(sniff(&pad(b"IIRO\x08\0\0\0"), None), Some(Kind::Raster("orf")));
        assert_eq!(sniff(&pad(b"IIU\0\x18\0\0\0"), None), Some(Kind::Raster("rw2")));
        assert_eq!(sniff(&pad(b"gimp xcf v011\0"), None), Some(Kind::Raster("xcf")));
        assert_eq!(sniff(&pad(&[0xff, 0x0a]), None), Some(Kind::Raster("jxl")));
        assert_eq!(sniff(&pad(b"\0\0\0\x0cJXL \r\n\x87\n"), None), Some(Kind::Raster("jxl")));
        assert_eq!(sniff(&pad(b"\0\0\0\x0cjP  \r\n\x87\n"), None), Some(Kind::Raster("jp2")));
        assert_eq!(sniff(&pad(&[0x76, 0x2f, 0x31, 0x01]), None), Some(Kind::Raster("exr")));
        assert_eq!(sniff(&pad(b"#?RADIANCE\n"), None), Some(Kind::Raster("hdr")));
        assert_eq!(sniff(&pad(b"DDS \x7c\0\0\0"), None), Some(Kind::Raster("dds")));
        let mut dcm = vec![0u8; 128];
        dcm.extend(b"DICM\x02\0\0\0");
        assert_eq!(sniff(&dcm, None), Some(Kind::Raster("dcm")));
        let tga = [&[0u8, 0, 2][..], &[0u8; 40][..]].concat();
        assert_eq!(sniff(&tga, Some("sprite.TGA")), Some(Kind::Raster("tga")));
        assert_eq!(sniff(&tga, Some("sprite.bin")), None);
        assert_eq!(sniff(b"plain text named tga", Some("notes.tga")), None);
    }

    #[test]
    fn markup_for_pandoc() {
        let epub = [zip_entry("mimetype", 0, b"application/epub+zip"), zip_entry("META-INF/container.xml", 8, &[1; 9])].concat();
        assert_eq!(sniff(&epub, None), Some(Kind::Markup("epub")));
        assert_eq!(sniff(&epub, Some("book.docx")), Some(Kind::Markup("epub")), "content beats the name");
        assert_eq!(sniff(b"{\"cells\": [], \"nbformat\": 4}", Some("analysis.ipynb")), Some(Kind::Markup("ipynb")));
        assert_eq!(sniff("# 标题\n\n正文段落。".as_bytes(), Some("说明.MD")), Some(Kind::Markup("gfm")));
        assert_eq!(sniff(b"# heading\n\ntext", Some("README.markdown")), Some(Kind::Markup("gfm")));
        assert_eq!(sniff(b"* heading\n** todo item", Some("notes.org")), None, "not sure to render whole");
        assert_eq!(sniff(b"Title\n=====\n\ntext", Some("doc.rst")), None);
        assert_eq!(sniff(b"# heading\n\ntext", Some("notes.txt")), None);
        assert_eq!(sniff(b"\0\x01\x02binary\0\0\0\0\0", Some("fake.md")), None, "the name alone is not enough");
        let cut = "汉字汉字汉字汉字".as_bytes();
        assert_eq!(sniff(&cut[..cut.len() - 1], Some("cut.md")), Some(Kind::Markup("gfm")), "a head may end inside a character");
    }

    fn zip_entry(name: &str, method: u16, data: &[u8]) -> Vec<u8> {
        let mut h = b"PK\x03\x04\x14\0\x08\x08".to_vec();
        h.extend(method.to_le_bytes());
        h.extend([0u8; 16]);
        h.extend((name.len() as u16).to_le_bytes());
        h.extend([0u8; 2]);
        h.extend(name.as_bytes());
        h.extend(data);
        h
    }

    #[test]
    fn documents() {
        let docx = [zip_entry("_rels/.rels", 8, &[0x9c; 40]), zip_entry("word/_rels/document.xml.rels", 8, &[1; 9]), zip_entry("word/document.xml", 8, &[2; 9])].concat();
        assert_eq!(sniff(&docx, None), Some(Kind::Document("docx")));
        let xlsx = [zip_entry("[Content_Types].xml", 8, &[3; 9]), zip_entry("xl/workbook.xml", 8, &[4; 9])].concat();
        assert_eq!(sniff(&xlsx, Some("data.bin")), Some(Kind::Document("xlsx")));
        let odp = zip_entry("mimetype", 0, b"application/vnd.oasis.opendocument.presentationPK");
        assert_eq!(sniff(&odp, None), Some(Kind::Document("odp")));
        let odb = zip_entry("mimetype", 0, b"application/vnd.oasis.opendocument.base");
        assert_eq!(sniff(&odb, None), None);
        let other = zip_entry("readme.txt", 0, b"hello world");
        assert_eq!(sniff(&other, None), None);
        assert_eq!(sniff(&other, Some("Q3 Report.DOCX")), Some(Kind::Document("docx")), "the name settles a zip the head leaves open");
        assert_eq!(sniff(b"{\\rtf1\\ansi\\deff0 hello}", None), Some(Kind::Document("rtf")));

        let mut ole = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
        ole.resize(512 + 128 * 3, 0);
        assert_eq!(sniff(&ole, None), None, "an OLE2 file is not necessarily a document (msi, msg)");
        assert_eq!(sniff(&ole, Some("old.PPT")), Some(Kind::Document("ppt")));
        assert_eq!(sniff(&ole, Some("setup.msi")), None);
        let name: Vec<u8> = "PowerPoint Document".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(sniff(&ole, Some("报告.wps")), Some(Kind::Document("doc")));
        assert_eq!(sniff(&ole, Some("表.ET")), Some(Kind::Document("xls")));
        assert_eq!(sniff(&ole, Some("演示.dps")), Some(Kind::Document("ppt")));
        assert_eq!(sniff(&ole, Some("flyer.pub")), Some(Kind::Document("pub")));
        assert_eq!(sniff(&ole, Some("문서.hwp")), None, "HWP 5 has no converter");
        assert_eq!(sniff(b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05", None), Some(Kind::Document("hwp")));
        ole[30] = 9;
        ole[48..52].copy_from_slice(&0x1234u32.to_le_bytes());
        assert_eq!(ole_directory_offset(&ole), Some((0x1234 + 1) * 512));
        ole[30] = 12;
        assert_eq!(ole_directory_offset(&ole), Some((0x1234 + 1) * 4096));
        ole[48..52].copy_from_slice(&0xffff_fffeu32.to_le_bytes());
        assert_eq!(ole_directory_offset(&ole), None);

        assert_eq!(sniff(&other, Some("macro.docm")), Some(Kind::Document("docx")));
        assert_eq!(sniff(&other, Some("show.ppsx")), Some(Kind::Document("pptx")));
        assert_eq!(sniff(&other, Some("book.xlsm")), Some(Kind::Document("xlsx")));
        assert_eq!(sniff(&other, Some("letter.pages")), Some(Kind::Document("pages")));
        let iwork = [zip_entry("Index/Document.iwa", 0, &[5; 9]), zip_entry("Index/Tables/DataList.iwa", 0, &[6; 9])].concat();
        assert_eq!(sniff(&iwork, Some("letter.pages")), Some(Kind::Document("pages")));
        assert_eq!(sniff(&iwork, Some("budget.numbers")), Some(Kind::Document("numbers")));
        assert_eq!(sniff(&iwork, None), None);
        let vsdx = [zip_entry("[Content_Types].xml", 8, &[3; 9]), zip_entry("visio/document.xml", 8, &[4; 9])].concat();
        assert_eq!(sniff(&vsdx, None), Some(Kind::Document("vsdx")));
        assert_eq!(sniff(b"\xffWPC\x10\0\0\0\x01\x0a\0\0", None), Some(Kind::Document("wpd")));
        assert!(is_sheet("xls") && is_sheet("ods") && is_sheet("numbers") && !is_sheet("xlsx") && !is_sheet("doc"));
        assert_eq!((office_of("vsdx"), office_of("numbers"), office_of("hwp")), (Office::Impress, Office::Calc, Office::Writer));

        ole[30] = 9;
        ole[640..640 + name.len()].copy_from_slice(&name);
        ole[640 + 64..640 + 66].copy_from_slice(&((name.len() + 2) as u16).to_le_bytes());
        assert_eq!(sniff(&ole, Some("deck.doc")), Some(Kind::Document("ppt")), "streams beat the name");
    }

    #[test]
    fn ffprobe_json() {
        let info = MediaInfo::parse(&json!({
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "5.000000"},
            "streams": [
                {"index": 0, "codec_type": "video", "codec_name": "hevc", "codec_tag_string": "hvc1", "width": 1280, "height": 720,
                 "pix_fmt": "yuv420p10le", "color_transfer": "arib-std-b67", "disposition": {"default": 1, "attached_pic": 0},
                 "side_data_list": [{"side_data_type": "Display Matrix", "rotation": -90}]},
                {"index": 1, "codec_type": "audio", "codec_name": "aac", "disposition": {"default": 1}}
            ],
            "stream_groups": [{"type": "Tile Grid", "disposition": {"default": 1}, "components": [{"width": 4032, "height": 3024,
                "horizontal_offset": 0, "vertical_offset": 0, "subcomponents": [{"stream_index": 0, "tile_horizontal_offset": 0, "tile_vertical_offset": 0}]}]}]
        }));
        assert_eq!(info.duration, Some(5.0));
        let v = info.video().unwrap();
        assert!(v.hdr() && v.rotation == -90 && v.codec_tag == "hvc1");
        assert_eq!(info.audio().unwrap().codec_name, "aac");
        assert_eq!(info.primary_grid().unwrap().tiles, vec![(0, 0, 0)]);
    }
}
