//! An image's size in pixels, read from its file header without decoding it.
//!
//! The editor draws a vault image at its true size before the image has loaded: the
//! placeholder holds the aspect ratio through the `<img>` width and height attributes,
//! so the text under an image no longer moves when it arrives. The in-memory size cache
//! only covered images seen since launch; this reads the size of every image a note
//! refers to when the note opens, from a few dozen bytes of each file (a JPEG walks its
//! segment headers by seeking, never reading the compressed data).
//!
//! The size reported is the one the webview displays: a JPEG's or a TIFF's EXIF
//! orientation, and an AVIF's or HEIF's rotation, swap the two sides when they turn the
//! picture by a quarter. An SVG has a size only when its root element states both sides
//! in pixels; one sized by a viewBox alone, or in relative units, reports none, and the
//! placeholder stays the default until it loads. Anything unreadable is `None`.

use std::io::{Read, Seek, SeekFrom};

/// How far into a file an ISO media header (AVIF, HEIF) or an SVG root element is
/// looked for: both sit at the very start of a well-formed file.
const HEAD_BYTES: usize = 64 * 1024;
const SVG_HEAD_BYTES: usize = 4 * 1024;
/// A JPEG's segments before the frame header: a bound on the walk for a damaged file.
const MAX_JPEG_SEGMENTS: usize = 256;

pub fn image_size<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    let mut head = [0u8; 32];
    let n = read_up_to(r, &mut head)?;
    let h = &head[..n];
    let size = if h.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(h)
    } else if h.starts_with(b"GIF87a") || h.starts_with(b"GIF89a") {
        (n >= 10).then(|| (u16le(h, 6) as u32, u16le(h, 8) as u32))
    } else if h.starts_with(&[0xFF, 0xD8]) {
        jpeg(r)
    } else if h.starts_with(b"RIFF") && n >= 30 && &h[8..12] == b"WEBP" {
        webp(h)
    } else if h.starts_with(b"BM") {
        bmp(h)
    } else if h.starts_with(b"II*\0") || h.starts_with(b"MM\0*") {
        tiff_size(r, 0)
    } else if h.starts_with(&[0, 0, 1, 0]) {
        ico(r)
    } else if n >= 12 && &h[4..8] == b"ftyp" {
        iso_media(r)
    } else {
        svg(r)
    };
    size.filter(|&(w, h)| w > 0 && h > 0)
}

fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> Option<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
    Some(n)
}

/// `len` bytes from `at`, or fewer at the end of the file.
fn read_at<R: Read + Seek>(r: &mut R, at: u64, len: usize) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(at)).ok()?;
    let mut buf = vec![0u8; len];
    let n = read_up_to(r, &mut buf)?;
    buf.truncate(n);
    Some(buf)
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u16be(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn u32be(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn png(h: &[u8]) -> Option<(u32, u32)> {
    // the signature, then the IHDR chunk: length, type, width, height
    (h.len() >= 24 && &h[12..16] == b"IHDR").then(|| (u32be(h, 16), u32be(h, 20)))
}

fn webp(h: &[u8]) -> Option<(u32, u32)> {
    match &h[12..16] {
        // lossy: the frame tag (3 bytes) and the start code (3) come first
        b"VP8 " => Some(((u16le(h, 26) & 0x3FFF) as u32, (u16le(h, 28) & 0x3FFF) as u32)),
        // lossless: after the signature byte, two 14-bit fields holding the sides less one
        b"VP8L" => {
            let (b0, b1, b2, b3) = (h[21] as u32, h[22] as u32, h[23] as u32, h[24] as u32);
            Some((1 + (((b1 & 0x3F) << 8) | b0), 1 + (((b3 & 0x0F) << 10) | (b2 << 2) | ((b1 & 0xC0) >> 6))))
        }
        // extended: the canvas size, 24-bit sides less one
        b"VP8X" => {
            let w = 1 + (h[24] as u32 | (h[25] as u32) << 8 | (h[26] as u32) << 16);
            let hh = 1 + (h[27] as u32 | (h[28] as u32) << 8 | (h[29] as u32) << 16);
            Some((w, hh))
        }
        _ => None,
    }
}

fn bmp(h: &[u8]) -> Option<(u32, u32)> {
    if h.len() < 26 {
        return None;
    }
    let header = u32::from_le_bytes([h[14], h[15], h[16], h[17]]);
    if header == 12 {
        // the old core header: 16-bit sides
        return Some((u16le(h, 18) as u32, u16le(h, 20) as u32));
    }
    let w = i32::from_le_bytes([h[18], h[19], h[20], h[21]]);
    // a negative height is a top-down bitmap, not a smaller one
    let hh = i32::from_le_bytes([h[22], h[23], h[24], h[25]]);
    Some((w.unsigned_abs(), hh.unsigned_abs()))
}

/// An icon: the largest of its entries (a side of 0 means 256).
fn ico<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    let head = read_at(r, 0, 6)?;
    if head.len() < 6 {
        return None;
    }
    let count = u16le(&head, 4).min(64) as usize;
    let dir = read_at(r, 6, count * 16)?;
    dir.as_chunks::<16>()
        .0
        .iter()
        .map(|e| {
            let side = |b: u8| if b == 0 { 256 } else { b as u32 };
            (side(e[0]), side(e[1]))
        })
        .max_by_key(|&(w, h)| w * h)
}

fn jpeg<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    let mut at = 2u64;
    let mut quarter_turn = false;
    for _ in 0..MAX_JPEG_SEGMENTS {
        let mark = read_at(r, at, 4)?;
        if mark.len() < 2 || mark[0] != 0xFF {
            return None;
        }
        let kind = mark[1];
        // fill bytes before a marker
        if kind == 0xFF {
            at += 1;
            continue;
        }
        // markers with no length: the standalone ones, and the restart markers
        if kind == 0x01 || (0xD0..=0xD9).contains(&kind) {
            at += 2;
            continue;
        }
        if mark.len() < 4 {
            return None;
        }
        let len = u16be(&mark, 2) as u64;
        // a start-of-frame (every C0–CF but the tables C4, C8, CC): precision, height, width
        if (0xC0..=0xCF).contains(&kind) && !matches!(kind, 0xC4 | 0xC8 | 0xCC) {
            let sof = read_at(r, at + 4, 5)?;
            if sof.len() < 5 {
                return None;
            }
            let (h, w) = (u16be(&sof, 1) as u32, u16be(&sof, 3) as u32);
            return Some(if quarter_turn { (h, w) } else { (w, h) });
        }
        if kind == 0xE1 && !quarter_turn {
            let sig = read_at(r, at + 4, 6)?;
            if sig == b"Exif\0\0" {
                quarter_turn = tiff_ifd0(r, at + 10).is_some_and(|t| t.quarter_turn());
            }
        }
        at += 2 + len;
    }
    None
}

/// The first image directory of a TIFF structure at `base` (a TIFF file, or the EXIF
/// block inside a JPEG): its width, height and orientation, where present.
#[derive(Default)]
struct Ifd0 {
    width: Option<u32>,
    height: Option<u32>,
    orientation: Option<u16>,
}

impl Ifd0 {
    /// Orientations 5–8 are the four that turn the picture by a quarter.
    fn quarter_turn(&self) -> bool {
        matches!(self.orientation, Some(5..=8))
    }
}

fn tiff_ifd0<R: Read + Seek>(r: &mut R, base: u64) -> Option<Ifd0> {
    let head = read_at(r, base, 8)?;
    if head.len() < 8 {
        return None;
    }
    let le = match &head[..4] {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_ = |b: &[u8], at: usize| if le { u16le(b, at) } else { u16be(b, at) };
    let u32_ = |b: &[u8], at: usize| {
        let a = [b[at], b[at + 1], b[at + 2], b[at + 3]];
        if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        }
    };
    let ifd = base + u32_(&head, 4) as u64;
    let count = read_at(r, ifd, 2)?;
    if count.len() < 2 {
        return None;
    }
    let n = (u16_(&count, 0) as usize).min(512);
    let entries = read_at(r, ifd + 2, n * 12)?;
    let mut out = Ifd0::default();
    for e in entries.as_chunks::<12>().0 {
        let (tag, kind) = (u16_(e, 0), u16_(e, 2));
        // a SHORT value sits in the first two bytes of the value field, a LONG fills it
        let value = match kind {
            3 => u16_(e, 8) as u32,
            4 => u32_(e, 8),
            _ => continue,
        };
        match tag {
            0x0100 => out.width = Some(value),
            0x0101 => out.height = Some(value),
            0x0112 => out.orientation = Some(value as u16),
            _ => {}
        }
    }
    Some(out)
}

fn tiff_size<R: Read + Seek>(r: &mut R, base: u64) -> Option<(u32, u32)> {
    let ifd = tiff_ifd0(r, base)?;
    let (w, h) = (ifd.width?, ifd.height?);
    Some(if ifd.quarter_turn() { (h, w) } else { (w, h) })
}

/// AVIF and HEIF: the largest `ispe` (image spatial extent) property in the header —
/// the primary image, not a thumbnail — turned by an `irot` of a quarter or three.
fn iso_media<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    let buf = read_at(r, 0, HEAD_BYTES)?;
    let mut best: Option<(u32, u32)> = None;
    let mut quarter_turn = false;
    for i in 4..buf.len().saturating_sub(4) {
        match &buf[i..i + 4] {
            // the box type, then version and flags (4), width (4), height (4)
            b"ispe" if i + 16 <= buf.len() => {
                let (w, h) = (u32be(&buf, i + 8), u32be(&buf, i + 12));
                if best.is_none_or(|(bw, bh)| (w as u64 * h as u64) > (bw as u64 * bh as u64)) {
                    best = Some((w, h));
                }
            }
            // the box type, then one byte whose low two bits count quarter turns
            b"irot" if i + 5 <= buf.len() => quarter_turn = buf[i + 4] & 0b11 == 1 || buf[i + 4] & 0b11 == 3,
            _ => {}
        }
    }
    best.map(|(w, h)| if quarter_turn { (h, w) } else { (w, h) })
}

fn svg<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    let buf = read_at(r, 0, SVG_HEAD_BYTES)?;
    let text = String::from_utf8_lossy(&buf);
    let start = text.find("<svg")?;
    let tag = &text[start..start + text[start..].find('>')?];
    Some((svg_length(tag, "width")?, svg_length(tag, "height")?))
}

/// An attribute of the root element as whole pixels: a bare number or one in `px`.
fn svg_length(tag: &str, name: &str) -> Option<u32> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().last();
        rest = &rest[at + name.len()..];
        // `width` as a whole attribute name, not the tail of `stroke-width`
        if !before.is_some_and(|c| c.is_whitespace()) {
            continue;
        }
        let value = rest.trim_start().strip_prefix('=')?.trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let value = &value[1..];
        let value = &value[..value.find(quote)?];
        let number = value.trim().strip_suffix("px").unwrap_or(value.trim());
        let px: f64 = number.parse().ok()?;
        return (px.is_finite() && px >= 1.0).then(|| px.round() as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn size(bytes: &[u8]) -> Option<(u32, u32)> {
        image_size(&mut Cursor::new(bytes.to_vec()))
    }

    #[test]
    fn png_reads_ihdr() {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert_eq!(size(&b), Some((640, 480)));
    }

    #[test]
    fn gif_and_bmp() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x20, 0x03, 0x58, 0x02]);
        assert_eq!(size(&gif), Some((800, 600)));
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 12]);
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&300i32.to_le_bytes());
        bmp.extend_from_slice(&(-200i32).to_le_bytes());
        assert_eq!(size(&bmp), Some((300, 200)));
    }

    fn jpeg_with(orientation: Option<u16>) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        if let Some(o) = orientation {
            // APP1: "Exif\0\0", then a big-endian TIFF header with one IFD entry
            let mut tiff = b"MM\0*".to_vec();
            tiff.extend_from_slice(&8u32.to_be_bytes());
            tiff.extend_from_slice(&1u16.to_be_bytes());
            tiff.extend_from_slice(&0x0112u16.to_be_bytes());
            tiff.extend_from_slice(&3u16.to_be_bytes());
            tiff.extend_from_slice(&1u32.to_be_bytes());
            tiff.extend_from_slice(&o.to_be_bytes());
            tiff.extend_from_slice(&[0, 0]);
            tiff.extend_from_slice(&0u32.to_be_bytes());
            let len = (2 + 6 + tiff.len()) as u16;
            b.extend_from_slice(&[0xFF, 0xE1]);
            b.extend_from_slice(&len.to_be_bytes());
            b.extend_from_slice(b"Exif\0\0");
            b.extend_from_slice(&tiff);
        }
        // a quantisation table to walk past, then the baseline frame header
        b.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x04, 0x00, 0x00]);
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&480u16.to_be_bytes());
        b.extend_from_slice(&640u16.to_be_bytes());
        b.extend_from_slice(&[0; 10]);
        b
    }

    #[test]
    fn jpeg_walks_to_the_frame_and_honours_orientation() {
        assert_eq!(size(&jpeg_with(None)), Some((640, 480)));
        assert_eq!(size(&jpeg_with(Some(1))), Some((640, 480)));
        assert_eq!(size(&jpeg_with(Some(6))), Some((480, 640)));
    }

    #[test]
    fn webp_three_forms() {
        let riff = |kind: &[u8], rest: &[u8]| {
            let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
            b.extend_from_slice(kind);
            b.extend_from_slice(&[0; 4]);
            b.extend_from_slice(rest);
            b.resize(40, 0);
            b
        };
        // lossy: frame tag, start code, then 14-bit sides
        let mut lossy = vec![0, 0, 0, 0x9D, 0x01, 0x2A];
        lossy.extend_from_slice(&400u16.to_le_bytes());
        lossy.extend_from_slice(&300u16.to_le_bytes());
        assert_eq!(size(&riff(b"VP8 ", &lossy)), Some((400, 300)));
        // lossless: 0x2F, then (w-1) and (h-1) packed in 14 bits each
        let (w, h) = (400u32 - 1, 300u32 - 1);
        let bits = w | (h << 14);
        let mut lossless = vec![0x2F];
        lossless.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(size(&riff(b"VP8L", &lossless)), Some((400, 300)));
        // extended: flags (4), then 24-bit sides less one
        let mut ext = vec![0; 4];
        ext.extend_from_slice(&[0x8F, 0x01, 0x00, 0x2B, 0x01, 0x00]);
        assert_eq!(size(&riff(b"VP8X", &ext)), Some((400, 300)));
    }

    #[test]
    fn avif_takes_the_largest_extent() {
        let mut b = vec![0, 0, 0, 0x18];
        b.extend_from_slice(b"ftypavif");
        b.resize(32, 0);
        for (w, h) in [(160u32, 90u32), (1920, 1080)] {
            b.extend_from_slice(&[0, 0, 0, 0x14]);
            b.extend_from_slice(b"ispe");
            b.extend_from_slice(&[0; 4]);
            b.extend_from_slice(&w.to_be_bytes());
            b.extend_from_slice(&h.to_be_bytes());
        }
        assert_eq!(size(&b), Some((1920, 1080)));
    }

    #[test]
    fn svg_needs_both_sides_in_pixels() {
        assert_eq!(size(br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="120" height="80px">"#), Some((120, 80)));
        assert_eq!(size(br#"<svg stroke-width="2" width='64.4' height='32'>"#), Some((64, 32)));
        assert_eq!(size(br#"<svg viewBox="0 0 100 50">"#), None);
        assert_eq!(size(br#"<svg width="100%" height="50%">"#), None);
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(size(b""), None);
        assert_eq!(size(b"not an image at all"), None);
        assert_eq!(size(&[0xFF, 0xD8, 0x00]), None);
    }
}
