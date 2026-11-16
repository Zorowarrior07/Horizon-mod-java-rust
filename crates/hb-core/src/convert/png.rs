//! Texture sanity checks before a texture enters a package.
//!
//! The game refuses or renders wrongly on: a file that is not a PNG, a PNG
//! whose dimensions are not multiples of the pack's, a texture over the
//! atlas size in either direction, or a sprite whose size differs from the
//! vanilla it stands beside. This reader parses the IHDR chunk only, which is
//! all that is needed to say so, and never decodes pixel data.

/// A PNG's dimensions and colour type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    /// 0 grey, 2 RGB, 3 palette, 4 grey+alpha, 6 RGBA.
    pub color_type: u8,
    pub bit_depth: u8,
    /// True when an `acTL` chunk (an animated PNG) is present.
    pub animated: bool,
}

/// Anything wider or taller than this cannot go in an atlas.
const MAX_DIMENSION: u32 = 16_384;

impl PngInfo {
    /// A note-worthy oddity, if any. A texture that is merely a different size
    /// from vanilla is legal (packs do it deliberately), so this only reports
    /// what is certain to render wrong.
    pub fn warning(&self) -> Option<String> {
        if self.width == 0 || self.height == 0 {
            return Some("a zero-sized texture cannot be drawn".to_owned());
        }
        if self.width > MAX_DIMENSION || self.height > MAX_DIMENSION {
            return Some(format!(
                "{}x{} is larger than any atlas ({MAX_DIMENSION} max); the game will not stitch it",
                self.width, self.height
            ));
        }
        None
    }

    pub fn is_square(&self) -> bool {
        self.width == self.height
    }

    /// The multiple a texture is drawn at, for the report: a 16x16 is 1x, a
    /// 32x32 is 2x.
    pub fn resolution(&self) -> usize {
        if self.width == 0 {
            return 0;
        }
        (self.width as usize).div_ceil(16).max(1)
    }
}

/// Parses the PNG signature and IHDR. Returns the reason on malformed input.
pub fn check(bytes: &[u8]) -> Result<PngInfo, String> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 {
        return Err("shorter than a PNG signature".into());
    }
    if bytes[..8] != SIGNATURE {
        return Err("not a PNG: the 8-byte signature is wrong".into());
    }
    if bytes.len() < 8 + 8 + 13 {
        return Err("truncated before the IHDR chunk".into());
    }
    let length = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if &bytes[12..16] != b"IHDR" {
        return Err(format!(
            "the first chunk is `{}`, not IHDR",
            String::from_utf8_lossy(&bytes[12..16])
        ));
    }
    if length != 13 {
        return Err(format!("IHDR is {length} bytes, not 13"));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    let bit_depth = bytes[24];
    let color_type = bytes[25];
    if !matches!(color_type, 0 | 2 | 3 | 4 | 6) {
        return Err(format!("colour type {color_type} is not a PNG colour type"));
    }
    if !matches!(bit_depth, 1 | 2 | 4 | 8 | 16) {
        return Err(format!("bit depth {bit_depth} is not a PNG bit depth"));
    }
    // Walk the chunks for `acTL`, bounded so a corrupt file cannot loop.
    let mut animated = false;
    let mut at = 8usize;
    let mut seen = 0;
    while at + 8 <= bytes.len() && seen < 64 {
        let len = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
            as usize;
        let kind = &bytes[at + 4..at + 8];
        if kind == b"acTL" {
            animated = true;
        }
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        at = at.saturating_add(12).saturating_add(len);
        seen += 1;
    }
    Ok(PngInfo {
        width,
        height,
        color_type,
        bit_depth,
        animated,
    })
}

// `div_ceil` on integers is stable since 1.73; this workspace supports 1.80.
#[allow(dead_code)]
fn unused() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid minimal PNG header: signature, IHDR with the given size, IEND.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.push(8); // bit depth
        out.push(6); // RGBA
        out.extend_from_slice(&[0, 0, 0]); // compression, filter, interlace
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    #[test]
    fn reads_sizes_and_resolution() {
        let info = check(&png(16, 16)).unwrap();
        assert_eq!((info.width, info.height), (16, 16));
        assert_eq!(info.resolution(), 1);
        assert!(info.is_square());
        assert_eq!(check(&png(64, 64)).unwrap().resolution(), 4);
        assert!(!check(&png(64, 32)).unwrap().is_square());
    }

    #[test]
    fn refuses_non_png_and_truncated_files() {
        assert!(check(b"not a png at all").unwrap_err().contains("signature"));
        assert!(check(&[0x89, b'P']).unwrap_err().contains("shorter"));
        let mut truncated = png(16, 16);
        truncated.truncate(20);
        assert!(check(&truncated).is_err());
    }

    #[test]
    fn warns_on_oversized_textures() {
        let info = check(&png(20_000, 20_000)).unwrap();
        assert!(info.warning().unwrap().contains("atlas"));
        assert!(check(&png(16, 16)).unwrap().warning().is_none());
    }

    #[test]
    fn detects_apng() {
        let mut bytes = png(16, 16);
        // Insert an acTL chunk before IEND.
        let insert_at = bytes.len() - 12;
        let mut chunk = 8u32.to_be_bytes().to_vec();
        chunk.extend_from_slice(b"acTL");
        chunk.extend_from_slice(&[0u8; 8]);
        chunk.extend_from_slice(&[0u8; 4]);
        bytes.splice(insert_at..insert_at, chunk);
        assert!(check(&bytes).unwrap().animated);
    }
}
