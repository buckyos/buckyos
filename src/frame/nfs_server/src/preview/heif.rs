//! The parts of a HEIF `meta` box ffmpeg 7.1 does not apply: the primary
//! item's `irot` / `imir` transforms (an iPhone portrait is stored landscape
//! with `irot`), its `colr` (HDR transfer), and whether an alpha auxiliary
//! image belongs to it. Tile grids themselves come from ffprobe stream groups.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    /// Counter-clockwise quarter turns (`irot`).
    Rotate(u8),
    /// `imir` axis: 0 = vertical axis (left↔right), 1 = horizontal axis (top↔bottom).
    Mirror(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nclx {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeifInfo {
    /// `ispe` of the primary item, before transforms.
    pub width: u32,
    pub height: u32,
    pub transforms: Vec<Transform>,
    pub grid: bool,
    pub has_alpha: bool,
    pub nclx: Option<Nclx>,
}

impl HeifInfo {
    pub fn display_size(&self) -> (u32, u32) {
        let turns: u32 = self.transforms.iter().map(|t| if let Transform::Rotate(q) = t { *q as u32 } else { 0 }).sum();
        if turns % 2 == 1 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    /// PQ (16) or HLG (18) transfer.
    pub fn hdr(&self) -> bool {
        self.nclx.is_some_and(|c| c.transfer == 16 || c.transfer == 18)
    }
}

const ALPHA_AUX: [&str; 2] = ["urn:mpeg:hevc:2015:auxid:1", "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha"];

struct Box<'a> {
    kind: [u8; 4],
    body: &'a [u8],
}

fn boxes(mut data: &[u8]) -> Vec<Box<'_>> {
    let mut out = Vec::new();
    while data.len() >= 8 {
        let mut size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
        let kind = [data[4], data[5], data[6], data[7]];
        let mut header = 8u64;
        if size == 1 {
            if data.len() < 16 {
                break;
            }
            size = u64::from_be_bytes(data[8..16].try_into().unwrap());
            header = 16;
        } else if size == 0 {
            size = data.len() as u64;
        }
        if size < header {
            break;
        }
        let end = size.min(data.len() as u64) as usize;
        out.push(Box { kind, body: &data[header as usize..end] });
        if size as usize >= data.len() {
            break;
        }
        data = &data[size as usize..];
    }
    out
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn find<'a>(list: &'a [Box<'a>], kind: &[u8; 4]) -> Option<&'a Box<'a>> {
    list.iter().find(|b| &b.kind == kind)
}

/// Item types by id (`iinf` / `infe` v2–3).
fn item_types(iinf: &[u8]) -> Vec<(u32, [u8; 4])> {
    let version = iinf.first().copied().unwrap_or(0);
    let skip = if version == 0 { 6 } else { 8 };
    let mut out = Vec::new();
    for infe in boxes(iinf.get(skip..).unwrap_or_default()) {
        if &infe.kind != b"infe" {
            continue;
        }
        let b = infe.body;
        let v = b.first().copied().unwrap_or(0);
        let (id, at) = match v {
            2 => (be16(b, 4).map(u32::from), 6),
            3 => (be32(b, 4), 8),
            _ => continue,
        };
        if let (Some(id), Some(kind)) = (id, b.get(at + 2..at + 6)) {
            out.push((id, kind.try_into().unwrap()));
        }
    }
    out
}

/// `(type, from, [to])` references of `iref`.
fn references(iref: &[u8]) -> Vec<([u8; 4], u32, Vec<u32>)> {
    let wide = iref.first().copied().unwrap_or(0) == 1;
    let mut out = Vec::new();
    for r in boxes(iref.get(4..).unwrap_or_default()) {
        let b = r.body;
        let id = |at: usize| if wide { be32(b, at) } else { be16(b, at).map(u32::from) };
        let step = if wide { 4 } else { 2 };
        let (Some(from), Some(count)) = (id(0), be16(b, step)) else { continue };
        let to = (0..count as usize).filter_map(|i| id(step + 2 + i * step)).collect();
        out.push((r.kind, from, to));
    }
    out
}

/// Property indices (1-based into `ipco`) associated with each item, in order.
fn associations(ipma: &[u8]) -> Vec<(u32, Vec<u16>)> {
    let version = ipma.first().copied().unwrap_or(0);
    let wide_index = ipma.get(3).is_some_and(|f| f & 1 == 1);
    let Some(count) = be32(ipma, 4) else { return Vec::new() };
    let mut at = 8;
    let mut out = Vec::new();
    for _ in 0..count {
        let item = if version < 1 { be16(ipma, at).map(u32::from) } else { be32(ipma, at) };
        let Some(item) = item else { break };
        at += if version < 1 { 2 } else { 4 };
        let Some(&n) = ipma.get(at) else { break };
        at += 1;
        let mut props = Vec::new();
        for _ in 0..n {
            let index = if wide_index { be16(ipma, at).map(|v| v & 0x7fff) } else { ipma.get(at).map(|v| (v & 0x7f) as u16) };
            at += if wide_index { 2 } else { 1 };
            if let Some(i) = index.filter(|i| *i > 0) {
                props.push(i);
            }
        }
        out.push((item, props));
    }
    out
}

fn aux_type(auxc: &[u8]) -> Option<&str> {
    let text = auxc.get(4..)?;
    let end = text.iter().position(|b| *b == 0).unwrap_or(text.len());
    std::str::from_utf8(&text[..end]).ok()
}

/// Parses the `meta` box of a HEIF file prefix; `None` when it is not HEIF.
pub fn parse(data: &[u8]) -> Option<HeifInfo> {
    let top = boxes(data);
    let ftyp = find(&top, b"ftyp")?;
    let brands: Vec<&[u8]> = std::iter::once(ftyp.body.get(0..4)?).chain(ftyp.body.get(8..).unwrap_or_default().chunks(4)).collect();
    if !brands.iter().any(|b| matches!(*b, b"mif1" | b"msf1" | b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"avif")) {
        return None;
    }
    let meta = find(&top, b"meta")?;
    let children = boxes(meta.body.get(4..)?);
    let pitm = find(&children, b"pitm")?.body;
    let primary = if pitm.first() == Some(&0) { be16(pitm, 4).map(u32::from)? } else { be32(pitm, 4)? };
    let iprp = boxes(find(&children, b"iprp")?.body);
    let ipco = boxes(find(&iprp, b"ipco")?.body);
    let assoc: Vec<(u32, Vec<u16>)> = iprp.iter().filter(|b| &b.kind == b"ipma").flat_map(|b| associations(b.body)).collect();
    let props_of = |item: u32| -> Vec<&Box> {
        assoc.iter().filter(|(i, _)| *i == item).flat_map(|(_, p)| p.iter()).filter_map(|i| ipco.get(*i as usize - 1)).collect()
    };
    let mut info = HeifInfo::default();
    for p in props_of(primary) {
        match &p.kind {
            b"ispe" => {
                info.width = be32(p.body, 4)?;
                info.height = be32(p.body, 8)?;
            }
            b"irot" => info.transforms.push(Transform::Rotate(p.body.first()? & 3)),
            b"imir" => info.transforms.push(Transform::Mirror(p.body.first()? & 1)),
            b"colr" if p.body.get(0..4) == Some(b"nclx") => {
                info.nclx = Some(Nclx {
                    primaries: be16(p.body, 4)?,
                    transfer: be16(p.body, 6)?,
                    matrix: be16(p.body, 8)?,
                    full_range: p.body.get(10).is_some_and(|b| b & 0x80 != 0),
                })
            }
            _ => {}
        }
    }
    if info.width == 0 || info.height == 0 {
        return None;
    }
    let types = find(&children, b"iinf").map(|b| item_types(b.body)).unwrap_or_default();
    info.grid = types.iter().any(|(id, t)| *id == primary && t == b"grid");
    let refs = find(&children, b"iref").map(|b| references(b.body)).unwrap_or_default();
    info.has_alpha = refs.iter().any(|(kind, from, to)| {
        kind == b"auxl" && to.contains(&primary) && props_of(*from).iter().any(|p| &p.kind == b"auxC" && aux_type(p.body).is_some_and(|t| ALPHA_AUX.contains(&t)))
    });
    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
        let mut b = vec![version];
        b.extend_from_slice(&flags.to_be_bytes()[1..]);
        b.extend_from_slice(body);
        bx(kind, &b)
    }

    fn sample(rotate: Option<u8>, alpha: bool, nclx_transfer: u16) -> Vec<u8> {
        let mut ftyp = b"heic\0\0\0\0mif1heic".to_vec();
        ftyp = bx(b"ftyp", &std::mem::take(&mut ftyp));
        let mut ispe = 4032u32.to_be_bytes().to_vec();
        ispe.extend_from_slice(&3024u32.to_be_bytes());
        let mut ipco = full(b"ispe", 0, 0, &ispe);
        let mut colr = b"nclx".to_vec();
        for v in [9u16, nclx_transfer, 9] {
            colr.extend_from_slice(&v.to_be_bytes());
        }
        colr.push(0x80);
        ipco.extend(bx(b"colr", &colr));
        let mut primary_props = vec![1u8, 2];
        if let Some(r) = rotate {
            ipco.extend(bx(b"irot", &[r]));
            primary_props.push(3);
        }
        let mut items = vec![(1u16, b"grid"), (2u16, b"hvc1")];
        let mut iref = Vec::new();
        let mut ipma_entries = vec![(1u16, primary_props.clone())];
        if alpha {
            let mut auxc = vec![0, 0, 0, 0];
            auxc.extend_from_slice(b"urn:mpeg:hevc:2015:auxid:1\0");
            ipco.extend(bx(b"auxC", &auxc));
            let alpha_index = if rotate.is_some() { 4 } else { 3 };
            items.push((3, b"hvc1"));
            ipma_entries.push((3, vec![alpha_index]));
            let mut r = 3u16.to_be_bytes().to_vec();
            r.extend_from_slice(&1u16.to_be_bytes());
            r.extend_from_slice(&1u16.to_be_bytes());
            iref = full(b"iref", 0, 0, &bx(b"auxl", &r));
        }
        let mut ipma = (ipma_entries.len() as u32).to_be_bytes().to_vec();
        for (item, props) in &ipma_entries {
            ipma.extend_from_slice(&item.to_be_bytes());
            ipma.push(props.len() as u8);
            ipma.extend(props.iter().map(|p| 0x80 | p));
        }
        let iprp = bx(b"iprp", &[bx(b"ipco", &ipco), full(b"ipma", 0, 0, &ipma)].concat());
        let mut infes = (items.len() as u16).to_be_bytes().to_vec();
        for (id, kind) in &items {
            let mut b = id.to_be_bytes().to_vec();
            b.extend_from_slice(&[0, 0]);
            b.extend_from_slice(*kind);
            b.push(0);
            infes.extend(full(b"infe", 2, 0, &b));
        }
        let iinf = full(b"iinf", 0, 0, &infes);
        let pitm = full(b"pitm", 0, 0, &1u16.to_be_bytes());
        let meta = full(b"meta", 0, 0, &[bx(b"hdlr", &[0; 24]), pitm, iinf, iref, iprp].concat());
        [ftyp, meta].concat()
    }

    #[test]
    fn plain_grid() {
        let info = parse(&sample(None, false, 13)).unwrap();
        assert_eq!((info.width, info.height), (4032, 3024));
        assert!(info.grid && !info.has_alpha && !info.hdr());
        assert_eq!(info.display_size(), (4032, 3024));
        assert_eq!(info.nclx.unwrap().primaries, 9);
    }

    #[test]
    fn portrait_alpha_hdr() {
        let info = parse(&sample(Some(3), true, 16)).unwrap();
        assert_eq!(info.transforms, vec![Transform::Rotate(3)]);
        assert_eq!(info.display_size(), (3024, 4032));
        assert!(info.has_alpha && info.hdr());
    }

    #[test]
    fn not_heif() {
        assert!(parse(b"\x89PNG\r\n\x1a\n0000").is_none());
        let mut mp4 = bx(b"ftyp", b"isom\0\0\0\0isomiso2mp41");
        mp4.extend(bx(b"moov", &[]));
        assert!(parse(&mp4).is_none());
    }
}
