//! What a source is, before (magic bytes: the planning decision) and after
//! (`ffprobe` JSON: the execution facts) a conversion starts.
//!
//! Office documents (Office import TODO §5.1): OOXML and ODF are zips told
//! apart by their entry names, the legacy binaries OLE2 compound files by their
//! stream names, RTF by `{\rtf`; the file name only settles what the first
//! MiB leaves open.

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

/// The import extension of an Office / ODF / RTF document.
fn document(head: &[u8], name: Option<&str>) -> Option<&'static str> {
    let ext = name.and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase());
    let hinted = |allowed: &[&'static str]| ext.as_deref().and_then(|e| allowed.iter().copied().find(|a| *a == e));
    if head.starts_with(b"{\\rtf") {
        return Some("rtf");
    }
    if head.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        let streams = [("WordDocument", "doc"), ("PowerPoint Document", "ppt"), ("Workbook", "xls"), ("Book", "xls")];
        return streams.iter().find(|(s, _)| ole_stream(head, s)).map(|(_, e)| *e).or_else(|| hinted(&["doc", "ppt", "xls"]));
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
    }
    let parts: [(&[u8], &'static str); 3] = [(b"word/document", "docx"), (b"ppt/presentation", "pptx"), (b"xl/workbook", "xlsx")];
    for (entry, _, _) in &entries {
        if let Some((_, e)) = parts.iter().find(|(p, _)| entry.starts_with(p) && entry.ends_with(b".xml")) {
            return Some(e);
        }
    }
    hinted(&["docx", "pptx", "xlsx", "odt", "odp", "ods", "odg"])
}

/// Planning by magic bytes (`name` only as a hint); `None` = no converter for this content.
pub fn sniff(head: &[u8], name: Option<&str>) -> Option<Kind> {
    if head.len() < 12 {
        return None;
    }
    if let Some(ext) = document(head, name) {
        return Some(Kind::Document(ext));
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
    None
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
        assert_eq!(sniff(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3", None), None);
        assert_eq!(sniff(b"PK\x03\x04\x14\0\0\0\0\0\0\0", None), None);
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
