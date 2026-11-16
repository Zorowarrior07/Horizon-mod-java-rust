//! CRC-32/ISO-HDLC, the variant zip uses.
//!
//! Table-driven with the table built once at first use. The slicing-by-8
//! variant would be faster; archives here are megabytes, not gigabytes, so
//! clarity wins.

use std::sync::OnceLock;

fn table() -> &'static [[u32; 256]; 8] {
    static TABLE: OnceLock<[[u32; 256]; 8]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [[0u32; 256]; 8];
        for (i, slot) in table[0].iter_mut().enumerate() {
            let mut crc = i as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
            *slot = crc;
        }
        for k in 1..8 {
            for i in 0..256 {
                let prev = table[k - 1][i];
                table[k][i] = table[0][(prev & 0xFF) as usize] ^ (prev >> 8);
            }
        }
        table
    })
}

/// CRC-32 of `bytes` (reflected, polynomial 0xEDB88320, initial 0, final
/// complement) — the value zip stores.
pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_update(0, bytes)
}

/// Continues a CRC over more data.
pub fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    let t = table();
    crc = !crc;
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let lo = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) ^ crc;
        crc = t[7][(lo & 0xFF) as usize]
            ^ t[6][((lo >> 8) & 0xFF) as usize]
            ^ t[5][((lo >> 16) & 0xFF) as usize]
            ^ t[4][(lo >> 24) as usize]
            ^ t[3][chunk[4] as usize]
            ^ t[2][chunk[5] as usize]
            ^ t[1][chunk[6] as usize]
            ^ t[0][chunk[7] as usize];
    }
    for &b in chunks.remainder() {
        crc = t[0][((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }

    #[test]
    fn incremental_matches_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(
            crc32_update(crc32_update(0, &data[..333]), &data[333..]),
            crc32(&data)
        );
    }

    #[test]
    fn all_lengths_agree_between_paths() {
        // Exercises the 8-byte chunked path against the byte path at every
        // remainder, which is where an off-by-one hides.
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let chunked = crc32(&data);
            let mut bytewise = !0u32;
            for &b in &data {
                bytewise = table()[0][((bytewise ^ b as u32) & 0xFF) as usize] ^ (bytewise >> 8);
            }
            assert_eq!(chunked, !bytewise, "len {len}");
        }
    }
}
