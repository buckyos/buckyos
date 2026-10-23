//! Media facts read from the bytes themselves: container, duration, frame
//! size and rate, and which tracks exist. Covers what video generation needs
//! to check (ISO-BMFF MP4/MOV, WAV, MP3, PNG/JPEG/WEBP/GIF) without an
//! external prober.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MediaContainer {
    Mp4,
    Mov,
    Wav,
    Mp3,
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl MediaContainer {
    pub(crate) fn mime(self) -> &'static str {
        match self {
            Self::Mp4 => "video/mp4",
            Self::Mov => "video/quicktime",
            Self::Wav => "audio/wav",
            Self::Mp3 => "audio/mpeg",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
        }
    }

    pub(crate) fn is_image(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::Webp | Self::Gif)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MediaFacts {
    pub container: MediaContainer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    pub has_video: bool,
    pub has_audio: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fragmented: bool,
}

impl MediaFacts {
    fn image(container: MediaContainer, width: u32, height: u32) -> Self {
        Self {
            container,
            duration_seconds: None,
            width: Some(width),
            height: Some(height),
            fps: None,
            has_video: false,
            has_audio: false,
            fragmented: false,
        }
    }

    pub(crate) fn metadata(&self) -> buckyos_api::VideoMetadata {
        buckyos_api::VideoMetadata {
            duration_seconds: self.duration_seconds,
            width: self.width,
            height: self.height,
            fps: self.fps,
            has_audio: Some(self.has_audio),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MediaErrorKind {
    Unrecognized,
    Malformed,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MediaError {
    pub kind: MediaErrorKind,
    pub message: String,
}

impl MediaError {
    fn new(kind: MediaErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn malformed(message: impl Into<String>) -> Self {
        Self::new(MediaErrorKind::Malformed, message)
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MediaError {}

pub(crate) fn probe(bytes: &[u8]) -> Result<MediaFacts, MediaError> {
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return probe_bmff(bytes);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return probe_wav(bytes);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return probe_webp(bytes);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return probe_png(bytes);
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return probe_jpeg(bytes);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        if bytes.len() < 10 {
            return Err(MediaError::malformed("GIF header is truncated"));
        }
        return Ok(MediaFacts::image(
            MediaContainer::Gif,
            u16::from_le_bytes([bytes[6], bytes[7]]) as u32,
            u16::from_le_bytes([bytes[8], bytes[9]]) as u32,
        ));
    }
    if bytes.starts_with(b"ID3") || mp3_frame(bytes, 0).is_some() {
        return probe_mp3(bytes);
    }
    Err(MediaError::new(
        MediaErrorKind::Unrecognized,
        "media container is not recognized",
    ))
}

#[derive(Clone, Copy, Debug)]
struct BoxHeader {
    kind: [u8; 4],
    start: usize,
    body: usize,
    end: usize,
}

fn read_box(bytes: &[u8], offset: usize, limit: usize) -> Result<Option<BoxHeader>, MediaError> {
    if offset == limit {
        return Ok(None);
    }
    if offset + 8 > limit {
        return Err(MediaError::malformed("ISO-BMFF box header is truncated"));
    }
    let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as u64;
    let kind: [u8; 4] = bytes[offset + 4..offset + 8].try_into().unwrap();
    let (size, header) = match size {
        0 => ((limit - offset) as u64, 8),
        1 => {
            if offset + 16 > limit {
                return Err(MediaError::malformed("ISO-BMFF large box is truncated"));
            }
            (
                u64::from_be_bytes(bytes[offset + 8..offset + 16].try_into().unwrap()),
                16,
            )
        }
        size => (size, 8),
    };
    let end = offset as u64 + size;
    if size < header as u64 || end > limit as u64 {
        return Err(MediaError::malformed(format!(
            "ISO-BMFF box `{}` exceeds its parent",
            String::from_utf8_lossy(&kind)
        )));
    }
    Ok(Some(BoxHeader {
        kind,
        start: offset,
        body: offset + header,
        end: end as usize,
    }))
}

fn children(bytes: &[u8], start: usize, end: usize) -> Result<Vec<BoxHeader>, MediaError> {
    let mut boxes = Vec::new();
    let mut offset = start;
    while let Some(header) = read_box(bytes, offset, end)? {
        offset = header.end;
        boxes.push(header);
    }
    Ok(boxes)
}

fn child(
    bytes: &[u8],
    parent: &BoxHeader,
    kind: &[u8; 4],
) -> Result<Option<BoxHeader>, MediaError> {
    Ok(children(bytes, parent.body, parent.end)?
        .into_iter()
        .find(|header| &header.kind == kind))
}

fn path(
    bytes: &[u8],
    parent: &BoxHeader,
    kinds: &[&[u8; 4]],
) -> Result<Option<BoxHeader>, MediaError> {
    let mut current = *parent;
    for kind in kinds {
        match child(bytes, &current, kind)? {
            Some(next) => current = next,
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, MediaError> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_be_bytes(value.try_into().unwrap()))
        .ok_or_else(|| MediaError::malformed("ISO-BMFF field is truncated"))
}

fn be_u64(bytes: &[u8], offset: usize) -> Result<u64, MediaError> {
    bytes
        .get(offset..offset + 8)
        .map(|value| u64::from_be_bytes(value.try_into().unwrap()))
        .ok_or_else(|| MediaError::malformed("ISO-BMFF field is truncated"))
}

/// `(timescale, duration)` of a `mvhd` or `mdhd` full box.
fn timed_header(bytes: &[u8], header: &BoxHeader) -> Result<(u32, u64), MediaError> {
    let version = *bytes
        .get(header.body)
        .ok_or_else(|| MediaError::malformed("ISO-BMFF header is truncated"))?;
    if version == 1 {
        Ok((
            be_u32(bytes, header.body + 20)?,
            be_u64(bytes, header.body + 24)?,
        ))
    } else {
        Ok((
            be_u32(bytes, header.body + 12)?,
            be_u32(bytes, header.body + 16)? as u64,
        ))
    }
}

struct TrackFacts {
    handler: [u8; 4],
    width: u32,
    height: u32,
    timescale: u32,
    duration: u64,
    samples: u64,
}

fn track_facts(bytes: &[u8], trak: &BoxHeader) -> Result<TrackFacts, MediaError> {
    let handler = path(bytes, trak, &[b"mdia", b"hdlr"])?
        .ok_or_else(|| MediaError::malformed("ISO-BMFF track has no handler"))?;
    let handler: [u8; 4] = bytes
        .get(handler.body + 8..handler.body + 12)
        .ok_or_else(|| MediaError::malformed("ISO-BMFF handler is truncated"))?
        .try_into()
        .unwrap();
    let (width, height) = match child(bytes, trak, b"tkhd")? {
        Some(tkhd) => {
            let version = bytes[tkhd.body];
            let base = tkhd.body + if version == 1 { 88 } else { 76 };
            (be_u32(bytes, base)? >> 16, be_u32(bytes, base + 4)? >> 16)
        }
        None => (0, 0),
    };
    let (timescale, duration) = match path(bytes, trak, &[b"mdia", b"mdhd"])? {
        Some(mdhd) => timed_header(bytes, &mdhd)?,
        None => (0, 0),
    };
    let mut samples = 0u64;
    if let Some(stts) = path(bytes, trak, &[b"mdia", b"minf", b"stbl", b"stts"])? {
        let count = be_u32(bytes, stts.body + 4)? as usize;
        for index in 0..count {
            samples += be_u32(bytes, stts.body + 8 + index * 8)? as u64;
        }
    }
    Ok(TrackFacts {
        handler,
        width,
        height,
        timescale,
        duration,
        samples,
    })
}

fn probe_bmff(bytes: &[u8]) -> Result<MediaFacts, MediaError> {
    let top = children(bytes, 0, bytes.len())?;
    let ftyp = top
        .iter()
        .find(|header| &header.kind == b"ftyp")
        .ok_or_else(|| MediaError::malformed("ISO-BMFF file has no ftyp"))?;
    let brand = bytes
        .get(ftyp.body..ftyp.body + 4)
        .ok_or_else(|| MediaError::malformed("ISO-BMFF brand is truncated"))?;
    let container = if brand == b"qt  " {
        MediaContainer::Mov
    } else {
        MediaContainer::Mp4
    };
    let moov = top
        .iter()
        .find(|header| &header.kind == b"moov")
        .ok_or_else(|| MediaError::malformed("ISO-BMFF file has no moov"))?;
    let fragmented =
        child(bytes, moov, b"mvex")?.is_some() || top.iter().any(|header| &header.kind == b"moof");
    let mut facts = MediaFacts {
        container,
        duration_seconds: None,
        width: None,
        height: None,
        fps: None,
        has_video: false,
        has_audio: false,
        fragmented,
    };
    if let Some(mvhd) = child(bytes, moov, b"mvhd")? {
        let (timescale, duration) = timed_header(bytes, &mvhd)?;
        if timescale > 0 && duration > 0 && duration != u32::MAX as u64 {
            facts.duration_seconds = Some(duration as f64 / timescale as f64);
        }
    }
    for trak in children(bytes, moov.body, moov.end)?
        .into_iter()
        .filter(|header| &header.kind == b"trak")
    {
        let track = track_facts(bytes, &trak)?;
        match &track.handler {
            b"vide" if !facts.has_video => {
                facts.has_video = true;
                facts.width = (track.width > 0).then_some(track.width);
                facts.height = (track.height > 0).then_some(track.height);
                if track.timescale > 0 && track.duration > 0 && track.samples > 0 {
                    let seconds = track.duration as f64 / track.timescale as f64;
                    facts.fps = Some(((track.samples as f64 / seconds) * 1000.0).round() / 1000.0);
                    if facts.duration_seconds.is_none() {
                        facts.duration_seconds = Some(seconds);
                    }
                }
            }
            b"soun" => facts.has_audio = true,
            _ => {}
        }
    }
    Ok(facts)
}

/// Removes every audio track from a non-fragmented MP4/MOV, rewriting chunk
/// offsets that point past the shrunken `moov`.
pub(crate) fn strip_audio_tracks(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    let facts = probe_bmff(bytes)?;
    if facts.fragmented {
        return Err(MediaError::new(
            MediaErrorKind::Unsupported,
            "audio tracks cannot be removed from a fragmented MP4",
        ));
    }
    if !facts.has_audio {
        return Ok(bytes.to_vec());
    }
    let top = children(bytes, 0, bytes.len())?;
    let moov = *top
        .iter()
        .find(|header| &header.kind == b"moov")
        .ok_or_else(|| MediaError::malformed("ISO-BMFF file has no moov"))?;
    let mut kept = Vec::new();
    for item in children(bytes, moov.body, moov.end)? {
        if &item.kind == b"trak" && &track_facts(bytes, &item)?.handler == b"soun" {
            continue;
        }
        kept.push(item);
    }
    if !kept.iter().any(|item| {
        &item.kind == b"trak" && track_facts(bytes, item).is_ok_and(|t| &t.handler == b"vide")
    }) {
        return Err(MediaError::new(
            MediaErrorKind::Unsupported,
            "the file has no video track left after removing audio",
        ));
    }
    let body_len: usize = kept.iter().map(|item| item.end - item.start).sum();
    let new_len = 8 + body_len;
    let new_moov_len = u32::try_from(new_len)
        .map_err(|_| MediaError::new(MediaErrorKind::Unsupported, "moov is too large"))?;
    let delta = new_len as i64 - (moov.end - moov.start) as i64;
    let mut new_moov = Vec::with_capacity(new_len);
    new_moov.extend_from_slice(&new_moov_len.to_be_bytes());
    new_moov.extend_from_slice(b"moov");
    for item in &kept {
        let start = new_moov.len();
        new_moov.extend_from_slice(&bytes[item.start..item.end]);
        if &item.kind == b"trak" {
            let relocated = BoxHeader {
                kind: item.kind,
                start,
                body: start + (item.body - item.start),
                end: start + (item.end - item.start),
            };
            patch_chunk_offsets(&mut new_moov, &relocated, moov.start as u64, delta)?;
        }
    }
    let mut output = Vec::with_capacity(bytes.len());
    output.extend_from_slice(&bytes[..moov.start]);
    output.extend_from_slice(&new_moov);
    output.extend_from_slice(&bytes[moov.end..]);
    Ok(output)
}

fn patch_chunk_offsets(
    moov: &mut [u8],
    trak: &BoxHeader,
    moov_start: u64,
    delta: i64,
) -> Result<(), MediaError> {
    let Some(stbl) = path(moov, trak, &[b"mdia", b"minf", b"stbl"])? else {
        return Ok(());
    };
    for table in children(moov, stbl.body, stbl.end)? {
        let wide = match &table.kind {
            b"stco" => false,
            b"co64" => true,
            _ => continue,
        };
        let count = be_u32(moov, table.body + 4)? as usize;
        let width = if wide { 8 } else { 4 };
        if table.body + 8 + count * width > table.end {
            return Err(MediaError::malformed("chunk offset table is truncated"));
        }
        for index in 0..count {
            let at = table.body + 8 + index * width;
            let offset = if wide {
                be_u64(moov, at)?
            } else {
                be_u32(moov, at)? as u64
            };
            if offset <= moov_start {
                continue;
            }
            let moved = offset as i64 + delta;
            if moved < 0 {
                return Err(MediaError::malformed("chunk offset moves before the file"));
            }
            if wide {
                moov[at..at + 8].copy_from_slice(&(moved as u64).to_be_bytes());
            } else {
                let moved = u32::try_from(moved)
                    .map_err(|_| MediaError::malformed("chunk offset overflows stco"))?;
                moov[at..at + 4].copy_from_slice(&moved.to_be_bytes());
            }
        }
    }
    Ok(())
}

fn probe_wav(bytes: &[u8]) -> Result<MediaFacts, MediaError> {
    let mut offset = 12;
    let mut byte_rate = None;
    let mut data_len = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let body = offset + 8;
        if id == b"fmt " {
            let rate = bytes
                .get(body + 8..body + 12)
                .ok_or_else(|| MediaError::malformed("WAV fmt chunk is truncated"))?;
            byte_rate = Some(u32::from_le_bytes(rate.try_into().unwrap()));
        } else if id == b"data" {
            data_len = Some(size.min(bytes.len().saturating_sub(body)));
            break;
        }
        offset = body + size + (size & 1);
    }
    let byte_rate = byte_rate
        .filter(|rate| *rate > 0)
        .ok_or_else(|| MediaError::malformed("WAV has no usable fmt chunk"))?;
    let data_len = data_len.ok_or_else(|| MediaError::malformed("WAV has no data chunk"))?;
    Ok(MediaFacts {
        container: MediaContainer::Wav,
        duration_seconds: Some(data_len as f64 / byte_rate as f64),
        width: None,
        height: None,
        fps: None,
        has_video: false,
        has_audio: true,
        fragmented: false,
    })
}

/// `(frame_len, samples, sample_rate)` of the MPEG audio frame at `offset`.
fn mp3_frame(bytes: &[u8], offset: usize) -> Option<(usize, u32, u32)> {
    let header = bytes.get(offset..offset + 4)?;
    if header[0] != 0xff || header[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version = (header[1] >> 3) & 0x03;
    let layer = (header[1] >> 1) & 0x03;
    let bitrate_index = (header[2] >> 4) as usize;
    let rate_index = ((header[2] >> 2) & 0x03) as usize;
    let padding = ((header[2] >> 1) & 0x01) as usize;
    if version == 1 || layer == 0 || bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    let mpeg1 = version == 3;
    const V1_L1: [u32; 15] = [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
    ];
    const V1_L2: [u32; 15] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
    ];
    const V1_L3: [u32; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const V2_L1: [u32; 15] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
    ];
    const V2_L23: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let kbps = match (mpeg1, layer) {
        (true, 3) => V1_L1[bitrate_index],
        (true, 2) => V1_L2[bitrate_index],
        (true, _) => V1_L3[bitrate_index],
        (false, 3) => V2_L1[bitrate_index],
        (false, _) => V2_L23[bitrate_index],
    };
    let sample_rate = [44_100, 48_000, 32_000][rate_index]
        >> match version {
            3 => 0,
            2 => 1,
            _ => 2,
        };
    let bitrate = kbps as usize * 1000;
    let (frame_len, samples) = match layer {
        3 => ((12 * bitrate / sample_rate as usize + padding) * 4, 384),
        2 => (144 * bitrate / sample_rate as usize + padding, 1152),
        _ if mpeg1 => (144 * bitrate / sample_rate as usize + padding, 1152),
        _ => (72 * bitrate / sample_rate as usize + padding, 576),
    };
    (frame_len >= 4).then_some((frame_len, samples, sample_rate))
}

fn probe_mp3(bytes: &[u8]) -> Result<MediaFacts, MediaError> {
    let mut offset = 0;
    if bytes.starts_with(b"ID3") {
        let header = bytes
            .get(6..10)
            .ok_or_else(|| MediaError::malformed("ID3 tag is truncated"))?;
        let size = header
            .iter()
            .fold(0usize, |size, byte| (size << 7) | (*byte as usize & 0x7f));
        let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
        offset = 10 + size + footer;
    }
    let mut frames = 0u64;
    let mut seconds = 0f64;
    while let Some((len, samples, rate)) = mp3_frame(bytes, offset) {
        if offset + len > bytes.len() {
            break;
        }
        frames += 1;
        seconds += samples as f64 / rate as f64;
        offset += len;
    }
    if frames < 2 {
        return Err(MediaError::malformed(
            "MP3 stream has no consecutive frames",
        ));
    }
    Ok(MediaFacts {
        container: MediaContainer::Mp3,
        duration_seconds: Some(seconds),
        width: None,
        height: None,
        fps: None,
        has_video: false,
        has_audio: true,
        fragmented: false,
    })
}

fn probe_png(bytes: &[u8]) -> Result<MediaFacts, MediaError> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return Err(MediaError::malformed("PNG has no IHDR"));
    }
    Ok(MediaFacts::image(
        MediaContainer::Png,
        be_u32(bytes, 16)?,
        be_u32(bytes, 20)?,
    ))
}

fn probe_jpeg(bytes: &[u8]) -> Result<MediaFacts, MediaError> {
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        if marker == 0xff {
            offset += 1;
            continue;
        }
        if matches!(marker, 0xd0..=0xd9 | 0x01) {
            offset += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let frame = bytes
                .get(offset + 5..offset + 9)
                .ok_or_else(|| MediaError::malformed("JPEG frame header is truncated"))?;
            return Ok(MediaFacts::image(
                MediaContainer::Jpeg,
                u16::from_be_bytes([frame[2], frame[3]]) as u32,
                u16::from_be_bytes([frame[0], frame[1]]) as u32,
            ));
        }
        offset += 2 + len;
    }
    Err(MediaError::malformed("JPEG has no frame header"))
}

fn probe_webp(bytes: &[u8]) -> Result<MediaFacts, MediaError> {
    let chunk = bytes
        .get(12..16)
        .ok_or_else(|| MediaError::malformed("WEBP chunk is truncated"))?;
    let at = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .ok_or_else(|| MediaError::malformed("WEBP header is truncated"))
    };
    let (width, height) = match chunk {
        b"VP8 " => {
            let frame = at(26..30)?;
            (
                (u16::from_le_bytes([frame[0], frame[1]]) & 0x3fff) as u32,
                (u16::from_le_bytes([frame[2], frame[3]]) & 0x3fff) as u32,
            )
        }
        b"VP8L" => {
            let bits = u32::from_le_bytes(at(21..25)?.try_into().unwrap());
            ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
        }
        b"VP8X" => {
            let size = at(24..30)?;
            (
                u32::from_le_bytes([size[0], size[1], size[2], 0]) + 1,
                u32::from_le_bytes([size[3], size[4], size[5], 0]) + 1,
            )
        }
        _ => return Err(MediaError::malformed("WEBP chunk is unknown")),
    };
    Ok(MediaFacts::image(MediaContainer::Webp, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO: &[u8] =
        include_bytes!("../../../../../test/jarvis_media_dv/assets/video_fresh.mp4");
    const WAV: &[u8] =
        include_bytes!("../../../../../test/jarvis_media_dv/assets/audio_speech.wav");
    const PNG: &[u8] =
        include_bytes!("../../../../../test/jarvis_media_dv/assets/image_primary.png");

    #[test]
    fn mp4_facts_come_from_the_movie_and_track_headers() {
        let facts = probe(VIDEO).unwrap();
        assert_eq!(facts.container, MediaContainer::Mp4);
        assert!((facts.duration_seconds.unwrap() - 5.055).abs() < 0.01);
        assert_eq!((facts.width, facts.height), (Some(960), Some(540)));
        assert!(facts.has_video && facts.has_audio && !facts.fragmented);
        let fps = facts.fps.unwrap();
        assert!(fps > 20.0 && fps < 61.0, "{fps}");
    }

    #[test]
    fn stripping_audio_keeps_video_samples_addressable() {
        let stripped = strip_audio_tracks(VIDEO).unwrap();
        assert!(stripped.len() < VIDEO.len());
        let facts = probe(&stripped).unwrap();
        assert!(facts.has_video && !facts.has_audio);
        assert_eq!(facts.width, Some(960));
        let original = probe(VIDEO).unwrap();
        assert_eq!(facts.fps, original.fps);

        let offsets = |bytes: &[u8]| {
            let top = children(bytes, 0, bytes.len()).unwrap();
            let moov = top.iter().find(|item| &item.kind == b"moov").unwrap();
            let trak = children(bytes, moov.body, moov.end)
                .unwrap()
                .into_iter()
                .find(|item| {
                    &item.kind == b"trak" && &track_facts(bytes, item).unwrap().handler == b"vide"
                })
                .unwrap();
            let stco = path(bytes, &trak, &[b"mdia", b"minf", b"stbl", b"stco"])
                .unwrap()
                .unwrap();
            let count = be_u32(bytes, stco.body + 4).unwrap() as usize;
            (0..count)
                .map(|index| be_u32(bytes, stco.body + 8 + index * 4).unwrap() as usize)
                .collect::<Vec<_>>()
        };
        let before = offsets(VIDEO);
        let after = offsets(&stripped);
        assert_eq!(before.len(), after.len());
        for (old, new) in before.iter().zip(&after) {
            assert_eq!(&VIDEO[*old..*old + 16], &stripped[*new..*new + 16]);
        }
        assert_eq!(strip_audio_tracks(&stripped).unwrap(), stripped);
    }

    #[test]
    fn wav_png_and_mp3_durations_and_sizes_are_measured() {
        let wav = probe(WAV).unwrap();
        assert_eq!(wav.container, MediaContainer::Wav);
        assert!(wav.duration_seconds.unwrap() > 1.0 && wav.has_audio);

        let png = probe(PNG).unwrap();
        assert_eq!(png.container, MediaContainer::Png);
        assert!(png.width.unwrap() > 0 && png.height.unwrap() > 0);

        // MPEG-1 Layer III, 128 kbps, 44.1 kHz: 417-byte frames of 1152 samples.
        let mut mp3 = Vec::new();
        for _ in 0..100 {
            let mut frame = vec![0u8; 417];
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
            mp3.extend_from_slice(&frame);
        }
        let facts = probe(&mp3).unwrap();
        assert_eq!(facts.container, MediaContainer::Mp3);
        assert!((facts.duration_seconds.unwrap() - 100.0 * 1152.0 / 44_100.0).abs() < 1e-6);
    }

    #[test]
    fn unknown_and_truncated_media_fail_with_a_kind() {
        assert_eq!(
            probe(b"plain text").unwrap_err().kind,
            MediaErrorKind::Unrecognized
        );
        assert_eq!(
            probe(&VIDEO[..64]).unwrap_err().kind,
            MediaErrorKind::Malformed
        );
    }
}
