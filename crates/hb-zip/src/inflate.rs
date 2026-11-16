//! DEFLATE (RFC 1951) decompression.
//!
//! Written for correctness and for good error messages, since the inputs are
//! third-party jars: every malformed case says what was wrong and where in
//! the stream. The output size is declared by the zip entry, so decompression
//! is bounded by construction and never allocates more than the entry claims.

use crate::ZipError;

fn err<T>(context: String) -> Result<T, ZipError> {
    Err(ZipError { context })
}

/// Huffman decoding table built from code lengths.
///
/// Canonical codes are decoded with a count/offset pair per length, which is
/// compact and branch-simple: for each bit read, walk lengths from the
/// shortest. Data is small enough that the simpler algorithm is the right one.
struct Huffman {
    /// Number of codes of each bit length 1..=15, at `counts[len]`.
    counts: [u16; 16],
    /// Symbols ordered by code.
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, ZipError> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len as usize > 15 {
                return err(format!("invalid Huffman code length {len}"));
            }
            counts[len as usize] += 1;
        }
        counts[0] = 0;
        // Reject over-subscribed sets; incomplete sets are legal for the
        // degenerate one-symbol case and are tolerated, as zlib does.
        let mut left: i32 = 1;
        for len in 1..16 {
            left <<= 1;
            left -= counts[len] as i32;
            if left < 0 {
                return err("over-subscribed Huffman code".to_owned());
            }
        }
        // Offsets of each length's first symbol.
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.iter().filter(|&&l| l != 0).count()];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                let slot = offsets[len as usize];
                if let Some(dest) = symbols.get_mut(slot as usize) {
                    *dest = symbol as u16;
                }
                offsets[len as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, ZipError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= reader.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                let at = (index + (code - first)) as usize;
                return match self.symbols.get(at) {
                    Some(&symbol) => Ok(symbol),
                    None => err(format!("Huffman symbol index {at} out of range")),
                };
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        err("invalid Huffman code".to_owned())
    }
}

struct BitReader<'a> {
    bytes: &'a [u8],
    at: usize,
    bit_buf: u32,
    bits_in_buf: u32,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            bit_buf: 0,
            bits_in_buf: 0,
        }
    }

    fn bits(&mut self, count: u32) -> Result<u32, ZipError> {
        debug_assert!(count <= 16);
        while self.bits_in_buf < count {
            let Some(&byte) = self.bytes.get(self.at) else {
                return err("deflate stream ended mid-symbol".to_owned());
            };
            self.at += 1;
            self.bit_buf |= (byte as u32) << self.bits_in_buf;
            self.bits_in_buf += 8;
        }
        let value = self.bit_buf & ((1u32 << count) - 1);
        self.bit_buf >>= count;
        self.bits_in_buf -= count;
        Ok(value)
    }

    fn align_to_byte(&mut self) {
        let drop = self.bits_in_buf % 8;
        self.bit_buf >>= drop;
        self.bits_in_buf -= drop;
    }

    /// Byte offset of the next unread byte after aligning.
    ///
    /// Bytes are refilled a whole byte at a time, so `at` always points past
    /// every byte pulled in, and the bytes still sitting in the buffer are a
    /// prefix of `bytes[at - buffered .. at]`.
    fn byte_position(&mut self) -> usize {
        self.align_to_byte();
        self.at - (self.bits_in_buf / 8) as usize
    }

    /// Consumes the `buffered` bytes that are already in the buffer, starting
    /// at `from`, without reading them again from the source.
    fn consume_buffered(&mut self, from: usize, buffered: usize) -> Option<&'a [u8]> {
        self.align_to_byte();
        let start = self.at.checked_sub((self.bits_in_buf / 8) as usize)?;
        if start != from {
            return None;
        }
        self.bit_buf = 0;
        self.bits_in_buf = 0;
        self.bytes.get(start..start + buffered)
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Decompresses a raw DEFLATE stream, refusing output longer than
/// `expected_size` (the zip entry's declared size).
pub fn decompress(input: &[u8], expected_size: usize, context: &str) -> Result<Vec<u8>, ZipError> {
    let mut out = Vec::with_capacity(expected_size.min(1 << 20));
    let mut reader = BitReader::new(input);
    loop {
        let final_block = reader.bits(1)? == 1;
        let kind = reader.bits(2)?;
        match kind {
            0 => stored_block(&mut reader, &mut out, expected_size)?,
            1 => {
                let (lit, dist) = fixed_tables();
                compressed_block(&mut reader, &mut out, expected_size, lit, dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_tables(&mut reader)?;
                compressed_block(&mut reader, &mut out, expected_size, &lit, &dist)?;
            }
            _ => return err(format!("{context}: invalid deflate block type 3")),
        }
        if final_block {
            break;
        }
    }
    Ok(out)
}

fn stored_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), ZipError> {
    reader.align_to_byte();
    let at = reader.byte_position();
    let buffered = reader.bits_in_buf as usize / 8;

    // The 4-byte header may already sit in the bit buffer; read whatever is
    // buffered from the buffer and only then pull from the source.
    let mut header = [0u8; 4];
    let from_buffer = buffered.min(4);
    if let Some(bytes) = reader.consume_buffered(at, from_buffer) {
        header[..from_buffer].copy_from_slice(bytes);
    } else {
        return err("stored block header is not where the bit buffer says".to_owned());
    }
    if from_buffer < 4 {
        let source = reader.bytes.get(at + from_buffer..).unwrap_or(&[]);
        let need = 4 - from_buffer;
        if source.len() < need {
            return err("stored block header truncated".to_owned());
        }
        header[from_buffer..].copy_from_slice(&source[..need]);
    }
    let len = u16::from_le_bytes([header[0], header[1]]);
    let nlen = u16::from_le_bytes([header[2], header[3]]);
    if len != !nlen {
        return err(format!(
            "stored block length check failed: LEN {:#06x}, NLEN {:#06x}",
            len, nlen
        ));
    }
    if out.len() + len as usize > limit {
        return err("output exceeds the declared size".to_owned());
    }
    let start = at + 4;
    let end = start + len as usize;
    if end > reader.bytes.len() {
        return err("stored block runs past the end of the stream".to_owned());
    }
    out.extend_from_slice(&reader.bytes[start..end]);
    reader.at = end;
    reader.bit_buf = 0;
    reader.bits_in_buf = 0;
    Ok(())
}

fn fixed_tables() -> (&'static Huffman, &'static Huffman) {
    use std::sync::OnceLock;
    static LIT: OnceLock<Huffman> = OnceLock::new();
    static DIST: OnceLock<Huffman> = OnceLock::new();
    let lit = LIT.get_or_init(|| {
        let mut lengths = [0u8; 288];
        for (i, slot) in lengths.iter_mut().enumerate() {
            *slot = match i {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
        }
        Huffman::new(&lengths).expect("fixed literal table is well formed")
    });
    let dist = DIST.get_or_init(|| {
        Huffman::new(&[5u8; 30]).expect("fixed distance table is well formed")
    });
    (lit, dist)
}

const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn dynamic_tables(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), ZipError> {
    let hlit = reader.bits(5)? as usize + 257;
    let hdist = reader.bits(5)? as usize + 1;
    let hclen = reader.bits(4)? as usize + 4;
    if hlit > 286 {
        return err(format!("HLIT {hlit} exceeds 286"));
    }
    if hdist > 30 {
        return err(format!("HDIST {hdist} exceeds 30"));
    }
    let mut code_lengths = [0u8; 19];
    for &position in CODE_LENGTH_ORDER.iter().take(hclen) {
        code_lengths[position] = reader.bits(3)? as u8;
    }
    let code_length_table = Huffman::new(&code_lengths)?;

    let mut lengths = vec![0u8; hlit + hdist];
    let mut at = 0;
    while at < lengths.len() {
        let symbol = code_length_table.decode(reader)?;
        match symbol {
            0..=15 => {
                lengths[at] = symbol as u8;
                at += 1;
            }
            16 => {
                if at == 0 {
                    return err("code length repeat with no previous length".to_owned());
                }
                let repeat = 3 + reader.bits(2)? as usize;
                let previous = lengths[at - 1];
                for _ in 0..repeat {
                    if at >= lengths.len() {
                        return err("code length repeat runs past the table".to_owned());
                    }
                    lengths[at] = previous;
                    at += 1;
                }
            }
            17 => {
                let repeat = 3 + reader.bits(3)? as usize;
                at += repeat;
                if at > lengths.len() {
                    return err("zero repeat runs past the table".to_owned());
                }
            }
            18 => {
                let repeat = 11 + reader.bits(7)? as usize;
                at += repeat;
                if at > lengths.len() {
                    return err("long zero repeat runs past the table".to_owned());
                }
            }
            other => return err(format!("invalid code length symbol {other}")),
        }
    }
    if lengths[256] == 0 {
        return err("literal table has no end-of-block code".to_owned());
    }
    let lit = Huffman::new(&lengths[..hlit])?;
    let mut dist_lengths = lengths[hlit..].to_vec();
    // A single distance code of length 0 is allowed when no matches occur;
    // some encoders emit all-zero distance tables for literals-only blocks.
    if dist_lengths.iter().all(|&l| l == 0) {
        dist_lengths[0] = 1;
    }
    let dist = Huffman::new(&dist_lengths)?;
    Ok((lit, dist))
}

fn compressed_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    limit: usize,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), ZipError> {
    loop {
        let symbol = lit.decode(reader)?;
        match symbol {
            0..=255 => {
                if out.len() >= limit {
                    return err("output exceeds the declared size".to_owned());
                }
                out.push(symbol as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let index = symbol as usize - 257;
                let length =
                    LENGTH_BASE[index] as usize + reader.bits(LENGTH_EXTRA[index] as u32)? as usize;
                let dist_symbol = dist.decode(reader)? as usize;
                if dist_symbol >= 30 {
                    return err(format!("invalid distance symbol {dist_symbol}"));
                }
                let distance = DIST_BASE[dist_symbol] as usize
                    + reader.bits(DIST_EXTRA[dist_symbol] as u32)? as usize;
                if distance > out.len() {
                    return err(format!(
                        "match distance {distance} reaches before the start of the output"
                    ));
                }
                if distance > 32768 {
                    return err(format!("match distance {distance} exceeds 32 KiB"));
                }
                if out.len() + length > limit {
                    return err("output exceeds the declared size".to_owned());
                }
                let start = out.len() - distance;
                // Overlapping copies must advance byte by byte; `copy_within`
                // would not repeat.
                for i in 0..length {
                    let byte = out[start + i];
                    out.push(byte);
                }
            }
            other => return err(format!("invalid literal/length symbol {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deflate;

    fn roundtrip(data: &[u8]) {
        let compressed = deflate::compress(data);
        let out = decompress(&compressed, data.len(), "test").unwrap();
        assert_eq!(out, data, "round trip of {} bytes", data.len());
    }

    #[test]
    fn round_trips_varied_input() {
        roundtrip(b"");
        roundtrip(b"a");
        roundtrip(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        roundtrip(b"abcabcabcabcabcabcabcabcabcabcabcabcabcabc");
        roundtrip(&(0..1000u32).map(|i| (i % 7) as u8).collect::<Vec<_>>());
        // Incompressible: random-ish bytes force stored blocks.
        let noise: Vec<u8> = (0..5000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        roundtrip(&noise);
        roundtrip("héllo wörld, 世界!".repeat(50).as_bytes());
    }

    #[test]
    fn round_trips_every_length_to_700() {
        for len in 0..700usize {
            let data: Vec<u8> = (0..len)
                .map(|i| ((i * 31) % 251) as u8 ^ (i % 3) as u8)
                .collect();
            roundtrip(&data);
        }
    }

    #[test]
    fn refuses_output_beyond_the_declared_size() {
        let compressed = deflate::compress(&vec![b'z'; 1000]);
        assert!(decompress(&compressed, 10, "t").is_err());
    }

    #[test]
    fn refuses_corrupt_streams() {
        // 0b110: final bit clear, block type 3 (reserved).
        assert!(decompress(&[0x06], 10, "t").is_err(), "block type 3");
        assert!(decompress(&[], 10, "t").is_err(), "no blocks at all");
        // Dynamic block header truncated straight after the block type.
        assert!(decompress(&[0x04], 10, "t").is_err(), "truncated header");
        // A match whose distance reaches before the start of the output must
        // be refused rather than reading whatever precedes it. Built with the
        // real encoder helpers so the stream is otherwise well formed: one
        // literal, then a 3-byte match at distance 8.
        let mut w = crate::deflate::BitWriter::new();
        w.bits(1, 1);
        w.bits(1, 2);
        crate::deflate::emit_literal(&mut w, b'a');
        crate::deflate::emit_match(&mut w, 3, 8);
        w.code(0, 7);
        let stream = w.finish();
        let error = decompress(&stream, 100, "t").unwrap_err();
        assert!(
            error.context.contains("before the start"),
            "got: {error}"
        );
        // Truncated valid data is refused too.
        let good = crate::deflate::compress(b"hello hello hello");
        assert!(decompress(&good[..good.len() / 2], 17, "t").is_err());
    }

    #[test]
    fn matches_known_deflate_output() {
        // "hello" compressed by zlib with a fixed Huffman block.
        let zlib_like: &[u8] = &[0xCB, 0x48, 0xCD, 0xC9, 0xC9, 0x07, 0x00];
        assert_eq!(decompress(zlib_like, 5, "t").unwrap(), b"hello");
    }
}
