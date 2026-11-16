//! DEFLATE (RFC 1951) compression.
//!
//! Strategy: one LZ77 pass with a hash chain over 3-byte prefixes and lazy
//! matching, emitted as fixed-Huffman blocks; stored blocks when that is
//! smaller. Fixed-Huffman costs roughly 5-10% against a dynamic table on JSON
//! but needs no code-length table, and `.hmod` payloads are a few hundred KiB
//! at most. Every block is checked to be smaller than storing, so the output
//! is never worse than uncompressed.
//!
//! Compressed output is decompressed by `inflate.rs` in tests, and by real
//! `unzip`/PowerShell `Expand-Archive` in `tests/windows_interop`.

const WINDOW: usize = 32 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const HASH_BITS: usize = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
const MAX_CHAIN: usize = 128;
const BLOCK: usize = 1 << 16;

pub(crate) struct BitWriter {
    out: Vec<u8>,
    bit_buf: u64,
    bits: u32,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self {
            out: Vec::new(),
            bit_buf: 0,
            bits: 0,
        }
    }

    /// Appends `count` bits of `value`, least-significant bit first (the order
    /// DEFLATE uses for lengths, distances and extra bits).
    pub(crate) fn bits(&mut self, value: u32, count: u32) {
        debug_assert!(count <= 32);
        self.bit_buf |= ((value as u64) & ((1u64 << count) - 1)) << self.bits;
        self.bits += count;
        while self.bits >= 8 {
            self.out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bits -= 8;
        }
    }

    /// Appends a Huffman code most-significant bit first.
    pub(crate) fn code(&mut self, code: u32, len: u32) {
        for i in (0..len).rev() {
            self.bits((code >> i) & 1, 1);
        }
    }

    /// Pads to a byte boundary and flushes.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.out.push((self.bit_buf & 0xFF) as u8);
        }
        self.out
    }

    fn align(&mut self) {
        while self.bits % 8 != 0 {
            self.bits(0, 1);
        }
    }
}

fn length_symbol(length: usize) -> (u16, u8) {
    const BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    // 258 maps to symbol 285 (the only value with a zero extra bit and a
    // base of 258), not to symbol 284's range.
    if length == 258 {
        return (285, 0);
    }
    let mut index = 0;
    for (i, &base) in BASE.iter().enumerate() {
        if (base as usize) <= length {
            index = i;
        }
    }
    (
        257 + index as u16,
        EXTRA[index],
    )
}

fn distance_symbol(distance: usize) -> (u8, u8) {
    const BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    let mut index = 0;
    for (i, &base) in BASE.iter().enumerate() {
        if (base as usize) <= distance {
            index = i;
        }
    }
    (index as u8, EXTRA[index])
}

pub(crate) fn emit_literal(writer: &mut BitWriter, byte: u8) {
    let symbol = byte as u32;
    match symbol {
        0..=143 => writer.code(0x30 + symbol, 8),
        _ => writer.code(0x190 + (symbol - 144), 9),
    }
}

pub(crate) fn emit_match(writer: &mut BitWriter, length: usize, distance: usize) {
    let (symbol, extra) = length_symbol(length);
    let symbol = symbol as u32;
    match symbol {
        257..=279 => writer.code(symbol - 256, 7),
        _ => writer.code(0xC0 + (symbol - 280), 8),
    }
    if extra > 0 {
        let base = length_base(symbol);
        writer.bits((length - base) as u32, extra as u32);
    }
    let (dist_symbol, dist_extra) = distance_symbol(distance);
    writer.code(dist_symbol as u32, 5);
    if dist_extra > 0 {
        writer.bits(
            (distance - distance_base(dist_symbol) as usize) as u32,
            dist_extra as u32,
        );
    }
}

fn length_base(symbol: u32) -> usize {
    const BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    BASE[(symbol - 257) as usize] as usize
}

fn distance_base(symbol: u8) -> u16 {
    const BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    BASE[symbol as usize]
}

fn hash3(data: &[u8], at: usize) -> usize {
    let a = data[at] as usize;
    let b = data[at + 1] as usize;
    let c = data[at + 2] as usize;
    ((a << 10) ^ (b << 5) ^ c) & (HASH_SIZE - 1)
}

/// Compresses `data` into a raw DEFLATE stream.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::new();
    let mut at = 0usize;
    let mut first_block = true;
    while at < data.len() || first_block {
        let end = (at + BLOCK).min(data.len());
        let last = end == data.len();
        let start_bits = writer.out.len();
        writer.bits(u32::from(last), 1);
        writer.bits(1, 2); // fixed Huffman
        let mut head = vec![usize::MAX; HASH_SIZE];
        let mut prev = vec![usize::MAX; end.max(1)];
        // The hash table starts at `at` so every block is independent, which
        // keeps the decoder's dictionary walk from reaching across a block.
        let mut position = at;
        while position < end {
            let remaining = end - position;
            let mut best_len = 0usize;
            let mut best_dist = 0usize;
            if remaining >= MIN_MATCH {
                let hash = hash3(data, position);
                let mut candidate = head[hash];
                let mut chain = 0;
                let max_len = remaining.min(MAX_MATCH);
                while candidate != usize::MAX && chain < MAX_CHAIN {
                    // Distance is bounded by the window and by the block start.
                    if position - candidate > WINDOW {
                        break;
                    }
                    let mut len = 0;
                    while len < max_len && data[candidate + len] == data[position + len] {
                        len += 1;
                    }
                    if len > best_len {
                        best_len = len;
                        best_dist = position - candidate;
                        if len >= max_len {
                            break;
                        }
                    }
                    candidate = prev[candidate];
                    chain += 1;
                }
                // Insert this position.
                prev[position] = head[hash];
                head[hash] = position;
            }
            // Lazy matching: if the next position has a longer match, emit a
            // literal here instead.
            let mut emit_match_here = best_len >= MIN_MATCH;
            if emit_match_here && best_len < MAX_MATCH && position + 1 < end {
                let next_remaining = end - (position + 1);
                if next_remaining >= MIN_MATCH {
                    let hash = hash3(data, position + 1);
                    let mut candidate = head[hash];
                    let mut chain = 0;
                    let max_len = next_remaining.min(MAX_MATCH);
                    let mut next_best = 0;
                    while candidate != usize::MAX && chain < 32 {
                        if position + 1 - candidate > WINDOW {
                            break;
                        }
                        let mut len = 0;
                        while len < max_len && data[candidate + len] == data[position + 1 + len] {
                            len += 1;
                        }
                        next_best = next_best.max(len);
                        if next_best > best_len {
                            break;
                        }
                        candidate = prev[candidate];
                        chain += 1;
                    }
                    if next_best > best_len {
                        emit_match_here = false;
                    }
                }
            }
            if emit_match_here {
                emit_match(&mut writer, best_len, best_dist);
                // Insert the covered positions so later matches can find them.
                for offset in 1..best_len {
                    let p = position + offset;
                    if p + MIN_MATCH <= end {
                        let hash = hash3(data, p);
                        prev[p] = head[hash];
                        head[hash] = p;
                    }
                }
                position += best_len;
            } else {
                emit_literal(&mut writer, data[position]);
                position += 1;
            }
        }
        writer.code(0, 7); // end of block (symbol 256)
        // Compare against storing this block; if storing is smaller, rewrite.
        let block_end_bits = writer.out.len() + usize::from(writer.bits > 0);
        let compressed_len = block_end_bits - start_bits;
        let stored_len = ((end - at) + 5).min(usize::MAX);
        if compressed_len >= stored_len {
            writer.out.truncate(start_bits);
            writer.bit_buf = 0;
            writer.bits = 0;
            write_stored(&mut writer, &data[at..end], last);
        }
        first_block = false;
        at = end;
        if end == 0 {
            break;
        }
    }
    writer.finish()
}

/// Writes `block` as one or more stored (uncompressed) blocks. An empty block
/// still has to terminate the stream, so it is written as a single final
/// zero-length stored block.
fn write_stored(writer: &mut BitWriter, block: &[u8], last: bool) {
    if block.is_empty() {
        writer.bits(u32::from(last), 1);
        writer.bits(0, 2);
        writer.align();
        write_raw_bytes(writer, &0u16.to_le_bytes());
        write_raw_bytes(writer, &(!0u16).to_le_bytes());
        return;
    }
    let mut at = 0;
    while at < block.len() {
        let take = (block.len() - at).min(65535);
        let final_block = last && at + take == block.len();
        writer.bits(u32::from(final_block), 1);
        writer.bits(0, 2);
        writer.align();
        let len = take as u16;
        write_raw_bytes(writer, &len.to_le_bytes());
        write_raw_bytes(writer, &(!len).to_le_bytes());
        write_raw_bytes(writer, &block[at..at + take]);
        at += take;
    }
}

fn write_raw_bytes(writer: &mut BitWriter, bytes: &[u8]) {
    debug_assert_eq!(writer.bits % 8, 0);
    writer.out.extend_from_slice(bytes);
    writer.bit_buf = 0;
    writer.bits = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inflate;

    fn roundtrip(data: &[u8]) -> usize {
        let compressed = compress(data);
        let out = inflate::decompress(&compressed, data.len(), "test").unwrap();
        assert_eq!(out, data, "round trip of {} bytes", data.len());
        compressed.len()
    }

    #[test]
    fn round_trips_empty_and_tiny() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| b'a' + (i % 26) as u8).collect();
            roundtrip(&data);
        }
    }

    #[test]
    fn round_trips_every_match_length() {
        // A run long enough to exercise all length codes and the 258 cap.
        for len in 3..=300usize {
            let mut data = b"seed".to_vec();
            data.extend(std::iter::repeat(b'x').take(len));
            roundtrip(&data);
        }
    }

    #[test]
    fn round_trips_all_distance_buckets() {
        for distance in [1usize, 2, 3, 4, 5, 8, 9, 16, 17, 32, 33, 128, 1024, 4096, 16384, 32768] {
            let mut data: Vec<u8> = (0..distance).map(|i| (i % 251) as u8).collect();
            let tail = data.clone();
            data.extend_from_slice(&tail[..tail.len().min(300)]);
            roundtrip(&data);
        }
    }

    #[test]
    fn round_trips_random_and_structured() {
        let noise: Vec<u8> = (0..20000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 11) as u8)
            .collect();
        roundtrip(&noise);
        let json = format!(
            "{{{}}}",
            (0..500)
                .map(|i| format!("\"k{i}\":{{\"type\":\"minecraft:crafting_shaped\",\"n\":{i}}}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        roundtrip(json.as_bytes());
    }

    #[test]
    fn compresses_repetitive_data_well() {
        let data = vec![b'{'; 100_000];
        let size = roundtrip(&data);
        assert!(size < 2_000, "100 KB of one byte -> {size} bytes");
    }

    #[test]
    fn does_not_expand_incompressible_data() {
        let noise: Vec<u8> = (0..5000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 9) as u8)
            .collect();
        let size = compress(&noise).len();
        assert!(
            size <= noise.len() + 6 + noise.len() / 1000,
            "stored fallback keeps overhead to block headers: {} vs {}",
            size,
            noise.len()
        );
    }

    #[test]
    fn round_trips_across_block_boundaries() {
        // Multiple 64 KiB blocks, each ending mid-match.
        for len in [65535usize, 65536, 65537, 131072, 200_000] {
            let data: Vec<u8> = (0..len).map(|i| ((i * 7) % 253) as u8).collect();
            roundtrip(&data);
        }
    }
}
