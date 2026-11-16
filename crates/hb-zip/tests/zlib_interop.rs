//! Interoperability with real zlib output.
//!
//! The fixtures in `tests/fixtures/` were produced by CPython's `zlib.compress`
//! (levels 1, 6 and 9), which emits *dynamic* Huffman blocks for most inputs
//! and *stored* blocks for incompressible ones. Our own encoder only emits
//! fixed and stored blocks, so these files are the only coverage of the dynamic
//! table path — and the only independent check that what we decode is what a
//! real DEFLATE implementation meant.
//!
//! `python_made.jar` is a zip written by CPython's `zipfile`, i.e. the layout a
//! jar from any real build tool has.

use hb_zip::{Archive, OutEntry};
use std::io::Cursor;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Tiny SHA-256, only so the fixture manifest can state what the expected
    // decompressed bytes are without shipping them a second time.
    let mut hex = String::new();
    for byte in sha256(bytes) {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Reads the `{file: {size, sha}}` manifest the fixture generator wrote.
fn expectations() -> Vec<(String, usize, String)> {
    let text = std::fs::read_to_string(fixtures().join("manifest.json")).unwrap();
    let mut out = Vec::new();
    // The manifest is `{ "<file>.deflate": { "size": N, "sha": "..." } }`,
    // pretty-printed; a line scan keeps this test free of a JSON dependency.
    let mut pending: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('"') && line.ends_with(": {") {
            pending = Some(line.trim_matches(|c| c == '"' || c == ',' || c == ' ' || c == '{' || c == ':').to_owned());
        } else if line.starts_with("\"size\":") {
            let size: usize = line
                .trim_start_matches("\"size\":")
                .trim_end_matches(',')
                .trim()
                .parse()
                .unwrap();
            if let Some(name) = pending.take() {
                pending = Some(format!("{name}\x1f{size}"));
            }
        } else if line.starts_with("\"sha\":") {
            let sha = line
                .trim_start_matches("\"sha\":")
                .trim_matches(|c: char| c == '"' || c == ',' || c == ' ')
                .to_owned();
            if let Some(combined) = pending.take() {
                let (name, size) = combined.split_once('\x1f').expect("size seen before sha");
                out.push((name.to_owned(), size.parse().unwrap(), sha));
            }
        }
    }
    out
}

#[test]
fn decodes_every_zlib_fixture_to_the_expected_bytes() {
    let expectations = expectations();
    assert!(
        expectations.len() >= 21,
        "expected at least 21 deflate fixtures, found {}",
        expectations.len()
    );
    for (file, size, sha) in expectations {
        let bytes = std::fs::read(fixtures().join(&file)).unwrap();
        let out = hb_zip::inflate::decompress(&bytes, size, &file).unwrap_or_else(|e| {
            panic!("{file}: {e}");
        });
        assert_eq!(out.len(), size, "{file}: size");
        assert_eq!(sha256_hex(&out), sha, "{file}: content");
    }
}

#[test]
fn decodes_a_python_made_zip() {
    let path = fixtures().join("python_made.jar");
    let files = hb_zip::read_zip_file(&path).unwrap();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"assets/demo/lang/en_us.json"), "{names:?}");
    assert!(names.contains(&"stored.txt"));
    let lang = files
        .iter()
        .find(|(n, _)| n == "assets/demo/lang/en_us.json")
        .unwrap();
    let text = String::from_utf8(lang.1.clone()).unwrap();
    assert!(text.contains("item.demo.thing"));
    let stored = files.iter().find(|(n, _)| n == "stored.txt").unwrap();
    assert_eq!(stored.1, b"plain stored bytes ".repeat(50));
}

#[test]
fn our_archives_are_readable_by_a_foreign_reader_layout() {
    // Checks the invariants a strict reader (Horizon's, or `unzip`) needs:
    // local headers, central directory and end record agree, and stored
    // entries have compressed == uncompressed size.
    let entries = vec![
        OutEntry::new("horizon-mod.toml", b"id = \"x\"\n".to_vec()),
        OutEntry::new("assets/x/lang/en_us.json", b"{\"a\":\"b\"}".to_vec()),
        OutEntry::new(
            "assets/x/textures/block/x.png",
            (0..=255u8).cycle().take(3000).collect::<Vec<u8>>(),
        ),
    ];
    let bytes = hb_zip::build(&entries).unwrap();
    let mut archive = Archive::new(Cursor::new(bytes.clone())).unwrap();
    assert_eq!(archive.entries().len(), 3);
    let mut rebuild = Vec::new();
    for entry in archive.entries().to_vec() {
        assert!(!entry.is_symlink() && !entry.is_executable());
        if entry.method == 0 {
            assert_eq!(entry.compressed_size, entry.uncompressed_size);
        }
        let data = archive.read(&entry.name).unwrap();
        rebuild.push(OutEntry::new(entry.name.clone(), data));
    }
    // Rebuilding from the read-back entries reproduces the same archive,
    // which only holds if every field we wrote is read back identically.
    assert_eq!(hb_zip::build(&rebuild).unwrap(), bytes);
}
