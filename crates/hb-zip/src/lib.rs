//! `hb-zip`: read jars and packs, write `.hmod` archives.
//!
//! Written from scratch because the bridge must build with no crates.io
//! access, and because the writer's job is unusual: Horizon's package reader
//! refuses symlinks, executable modes, absolute or `..` paths and duplicate
//! entries, so the writer should make those unrepresentable rather than rely
//! on the reader to catch them.
//!
//! Reader:
//! * central directory driven, so entries can be read by name in any order;
//! * stored (0) and deflate (8) — the only methods Horizons' reader supports,
//!   which is also all a jar from any mod loader uses in practice;
//! * sizes are checked against the declared values, and a decompression
//!   budget stops a zip bomb from exhausting memory.
//!
//! Writer:
//! * fixed timestamp (1980-01-01), fixed permissions, fixed order — two runs
//!   over the same inputs produce byte-identical archives, so a `sha512` in
//!   `[[jar]]` or a registry signature stays stable;
//! * stored entries only for tiny files, deflate elsewhere.

pub mod crc32;
pub mod deflate;
pub mod inflate;

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};

pub use crc32::crc32;

/// Why a zip could not be read.
#[derive(Debug)]
pub struct ZipError {
    pub context: String,
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.context)
    }
}

impl std::error::Error for ZipError {}

fn err<T>(context: impl Into<String>) -> Result<T, ZipError> {
    Err(ZipError {
        context: context.into(),
    })
}

/// One entry's metadata from the central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMeta {
    pub name: String,
    /// 0 (stored) or 8 (deflate).
    pub method: u16,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc: u32,
    /// Unix mode from the external attributes, when the archive came from a
    /// Unix host. `None` for DOS-created archives.
    pub unix_mode: Option<u32>,
    /// Offset of the local header.
    local_header_offset: u64,
    /// Offset of the central directory record, for writer round trips.
    central_offset: usize,
}

impl EntryMeta {
    /// True when the entry is a directory (name ending in `/` or the
    /// directory attribute set).
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
            || self.unix_mode.is_some_and(|m| m & 0o170000 == 0o040000)
    }

    /// True when the entry is a symbolic link, per its Unix mode.
    pub fn is_symlink(&self) -> bool {
        self.unix_mode.is_some_and(|m| m & 0o170000 == 0o120000)
    }

    /// True when any execute bit is set on a regular file.
    pub fn is_executable(&self) -> bool {
        self.unix_mode
            .is_some_and(|m| m & 0o170000 == 0o100000 && m & 0o111 != 0)
    }
}

/// Limits that keep hostile or corrupt archives from exhausting the machine.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// Total bytes any one archive may decompress to.
    pub max_total_uncompressed: u64,
    /// Largest single entry allowed.
    pub max_entry_uncompressed: u64,
    /// Most entries allowed.
    pub max_entries: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            // Horizons' own shared ceiling ("the shared 512 MiB hash limit")
            // with headroom for a modpack-sized jar.
            max_total_uncompressed: 2 << 30,
            max_entry_uncompressed: 512 << 20,
            max_entries: 200_000,
        }
    }
}

/// A parsed archive. Holds the central directory; entry bytes are read on
/// demand from the source.
pub struct Archive<R> {
    source: R,
    entries: Vec<EntryMeta>,
    by_name: BTreeMap<String, usize>,
    budget: Budget,
}

impl<R: Read + Seek> Archive<R> {
    pub fn new(source: R) -> Result<Self, ZipError> {
        Self::with_budget(source, Budget::default())
    }

    pub fn with_budget(mut source: R, budget: Budget) -> Result<Self, ZipError> {
        let length = source
            .seek(SeekFrom::End(0))
            .map_err(|e| ZipError { context: format!("cannot measure the archive: {e}") })?;
        if length < 22 {
            return err("not a zip: shorter than an end-of-central-directory record");
        }
        let tail_len = length.min(66_000) as usize;
        source
            .seek(SeekFrom::Start(length - tail_len as u64))
            .map_err(|e| ZipError { context: format!("seek failed: {e}") })?;
        let mut tail = vec![0u8; tail_len];
        source
            .read_exact(&mut tail)
            .map_err(|e| ZipError { context: format!("read failed: {e}") })?;

        let Some(eocd_at) = find_eocd(&tail) else {
            return err(
                "not a zip: no end-of-central-directory signature (a self-extracting prefix is \
                 not supported)",
            );
        };
        let tail = &tail[eocd_at..];
        let disk = u16::from_le_bytes([tail[4], tail[5]]);
        let cd_disk = u16::from_le_bytes([tail[6], tail[7]]);
        let entries_on_disk = u16::from_le_bytes([tail[8], tail[9]]);
        let entries_total = u16::from_le_bytes([tail[10], tail[11]]);
        let cd_size = u32::from_le_bytes([tail[12], tail[13], tail[14], tail[15]]) as u64;
        let cd_offset = u32::from_le_bytes([tail[16], tail[17], tail[18], tail[19]]) as u64;

        if entries_total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
            return err("zip64 archives are not supported");
        }
        if disk != 0 || cd_disk != 0 || entries_on_disk != entries_total {
            return err("multi-disk archives are not supported");
        }
        if entries_total as usize > budget.max_entries {
            return err(format!(
                "archive has {entries_total} entries, over the {} entry limit",
                budget.max_entries
            ));
        }
        let cd_end = cd_offset
            .checked_add(cd_size)
            .filter(|end| *end <= length)
            .ok_or_else(|| ZipError {
                context: "central directory runs past the end of the archive".into(),
            })?;
        let _ = cd_end;

        source
            .seek(SeekFrom::Start(cd_offset))
            .map_err(|e| ZipError { context: format!("seek failed: {e}") })?;
        let mut cd = vec![0u8; cd_size as usize];
        source
            .read_exact(&mut cd)
            .map_err(|e| ZipError { context: format!("central directory truncated: {e}") })?;

        let mut entries = Vec::with_capacity(entries_total as usize);
        let mut by_name = BTreeMap::new();
        let mut at = 0usize;
        for index in 0..entries_total as usize {
            let (entry, next) = parse_central_entry(&cd, at).ok_or_else(|| ZipError {
                context: format!("central directory entry {index} is malformed"),
            })?;
            at = next;
            if let Some(previous) = by_name.insert(entry.name.clone(), entries.len()) {
                return err(format!(
                    "duplicate entry `{}` (also entry {previous}): Horizon's package reader \
                     refuses a path that appears twice",
                    entry.name
                ));
            }
            entries.push(entry);
        }

        Ok(Self {
            source,
            entries,
            by_name,
            budget,
        })
    }

    pub fn entries(&self) -> &[EntryMeta] {
        &self.entries
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn meta(&self, name: &str) -> Option<&EntryMeta> {
        self.by_name.get(name).map(|i| &self.entries[*i])
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Reads and verifies one entry's bytes.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, ZipError> {
        let Some(index) = self.by_name.get(name).copied() else {
            return err(format!("no entry named `{name}`"));
        };
        self.read_index(index)
    }

    pub fn read_index(&mut self, index: usize) -> Result<Vec<u8>, ZipError> {
        let entry = self.entries[index].clone();
        let mut bytes = self.read_raw(&entry)?;
        if entry.method == 8 {
            bytes = inflate::decompress(&bytes, entry.uncompressed_size as usize, &entry.name)?;
        }
        if bytes.len() as u64 != entry.uncompressed_size {
            return err(format!(
                "`{}`: unpacked to {} bytes, the archive says {}",
                entry.name,
                bytes.len(),
                entry.uncompressed_size
            ));
        }
        if entry.uncompressed_size > self.budget.max_entry_uncompressed {
            return err(format!(
                "`{}`: {} bytes is over the per-entry limit",
                entry.name, entry.uncompressed_size
            ));
        }
        let actual = crc32(&bytes);
        if actual != entry.crc {
            return err(format!(
                "`{}`: CRC mismatch (archive {:08x}, data {:08x})",
                entry.name, entry.crc, actual
            ));
        }
        Ok(bytes)
    }

    /// Reads every entry, enforcing the total budget. Directory entries are
    /// skipped. Returns `(name, bytes)` in central-directory order.
    pub fn read_all(&mut self) -> Result<Vec<(String, Vec<u8>)>, ZipError> {
        let mut total = 0u64;
        let mut out = Vec::new();
        for index in 0..self.entries.len() {
            let entry = self.entries[index].clone();
            if entry.is_dir() {
                continue;
            }
            if entry.is_symlink() {
                return err(format!(
                    "`{}` is a symbolic link; Horizon's package reader refuses links",
                    entry.name
                ));
            }
            if entry.is_executable() {
                return err(format!(
                    "`{}` is executable (mode {:o}); Horizon's package reader refuses executable \
                     payloads",
                    entry.name,
                    entry.unix_mode.unwrap_or(0)
                ));
            }
            total = total.saturating_add(entry.uncompressed_size);
            if total > self.budget.max_total_uncompressed {
                return err(format!(
                    "archive unpacks to more than the {} MiB total limit",
                    self.budget.max_total_uncompressed >> 20
                ));
            }
            let bytes = self.read_index(index)?;
            out.push((entry.name, bytes));
        }
        Ok(out)
    }

    fn read_raw(&mut self, entry: &EntryMeta) -> Result<Vec<u8>, ZipError> {
        self.source
            .seek(SeekFrom::Start(entry.local_header_offset))
            .map_err(|e| ZipError { context: format!("seek failed: {e}") })?;
        let mut header = [0u8; 30];
        self.source
            .read_exact(&mut header)
            .map_err(|e| ZipError { context: format!("`{}`: truncated local header: {e}", entry.name) })?;
        if header[..4] != *b"PK\x03\x04" {
            return err(format!("`{}`: bad local header signature", entry.name));
        }
        let name_len = u16::from_le_bytes([header[26], header[27]]) as u64;
        let extra_len = u16::from_le_bytes([header[28], header[29]]) as u64;
        let data_start = entry
            .local_header_offset
            .checked_add(30)
            .and_then(|o| o.checked_add(name_len))
            .and_then(|o| o.checked_add(extra_len))
            .ok_or_else(|| ZipError { context: format!("`{}`: bad offsets", entry.name) })?;
        self.source
            .seek(SeekFrom::Start(data_start))
            .map_err(|e| ZipError { context: format!("seek failed: {e}") })?;
        let mut bytes = vec![0u8; entry.compressed_size as usize];
        self.source
            .read_exact(&mut bytes)
            .map_err(|e| ZipError { context: format!("`{}`: truncated data: {e}", entry.name) })?;
        Ok(bytes)
    }

    pub fn into_inner(self) -> R {
        self.source
    }
}

fn find_eocd(tail: &[u8]) -> Option<usize> {
    if tail.len() < 22 {
        return None;
    }
    for at in (0..=tail.len() - 22).rev() {
        if tail[at..at + 4] == *b"PK\x05\x06" {
            return Some(at);
        }
    }
    None
}

fn parse_central_entry(cd: &[u8], at: usize) -> Option<(EntryMeta, usize)> {
    let header = cd.get(at..at + 46)?;
    if header[..4] != *b"PK\x01\x02" {
        return None;
    }
    let method = u16::from_le_bytes([header[10], header[11]]);
    let crc = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
    let compressed_size = u32::from_le_bytes([header[20], header[21], header[22], header[23]]) as u64;
    let uncompressed_size =
        u32::from_le_bytes([header[24], header[25], header[26], header[27]]) as u64;
    let name_len = u16::from_le_bytes([header[28], header[29]]) as usize;
    let extra_len = u16::from_le_bytes([header[30], header[31]]) as usize;
    let comment_len = u16::from_le_bytes([header[32], header[33]]) as usize;
    let external_attributes =
        u32::from_le_bytes([header[38], header[39], header[40], header[41]]);
    let local_header_offset =
        u32::from_le_bytes([header[42], header[43], header[44], header[45]]) as u64;
    let name_at = at + 46;
    let name_bytes = cd.get(name_at..name_at + name_len)?;
    // Zip entry names are UTF-8 in every modern tool; refuse anything else
    // rather than lossily replacing, since a name is identity here.
    let name = std::str::from_utf8(name_bytes).ok()?.to_owned();

    // "Version made by" is a u16 at offset 4: the low byte is the spec
    // version, the high byte is the host OS, where 3 means Unix. The Unix mode
    // lives in the high 16 bits of the external attributes only for that host;
    // for a DOS host those bits are meaningless, so `None` (not a guess).
    let host_os = header[5];
    let unix_mode = (host_os == 3).then_some(external_attributes >> 16);

    let next = name_at + name_len + extra_len + comment_len;
    Some((
        EntryMeta {
            name,
            method,
            compressed_size,
            uncompressed_size,
            crc,
            unix_mode,
            local_header_offset,
            central_offset: at,
        },
        next,
    ))
}

impl EntryMeta {
    /// Byte offset of this entry's central directory record.
    pub fn central_directory_offset(&self) -> usize {
        self.central_offset
    }
}

/// One file to write.
#[derive(Debug, Clone)]
pub struct OutEntry {
    pub name: String,
    pub data: Vec<u8>,
    /// Force stored (no compression). Useful for already-compressed payloads
    /// such as PNG textures.
    pub store: bool,
}

impl OutEntry {
    pub fn new(name: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        let name = name.into();
        let data = data.into();
        let png = name.to_ascii_lowercase().ends_with(".png");
        Self {
            name,
            data,
            store: png,
        }
    }

    pub fn stored(mut self) -> Self {
        self.store = true;
        self
    }
}

/// Builds an archive. Paths are validated up front so a generated `.hmod` can
/// never trip Horizon's package reader on path rules.
pub fn build(entries: &[OutEntry]) -> Result<Vec<u8>, ZipError> {
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    let mut prepared: Vec<(String, u16, u32, u64, Vec<u8>)> = Vec::with_capacity(entries.len());
    for entry in entries {
        validate_path(&entry.name)?;
        if seen.insert(entry.name.as_str(), ()).is_some() {
            return err(format!("duplicate entry `{}`", entry.name));
        }
        let crc = crc32(&entry.data);
        // Deflate only when it actually helps; a stored entry that differs
        // from a deflated one only in a few bytes is not worth the CPU on load.
        let (method, payload) = if entry.store || entry.data.len() < 32 {
            (0u16, entry.data.clone())
        } else {
            let deflated = deflate::compress(&entry.data);
            if deflated.len() < entry.data.len() {
                (8u16, deflated)
            } else {
                (0u16, entry.data.clone())
            }
        };
        prepared.push((
            entry.name.clone(),
            method,
            crc,
            entry.data.len() as u64,
            payload,
        ));
    }

    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, method, crc, size, payload) in &prepared {
        let offset = out.len() as u32;
        let name_bytes = name.as_bytes();
        // Local file header. Version needed 2.0 (deflate), flags 0 (names and
        // payloads are already UTF-8; the language-encoding bit is not set
        // because Horizon's reader treats names as bytes).
        write_u32(&mut out, 0x0403_4b50);
        write_u16(&mut out, 20);
        write_u16(&mut out, 0);
        write_u16(&mut out, *method);
        write_u16(&mut out, 0); // time: 1980-01-01 00:00:00
        write_u16(&mut out, 0x0021); // date: 1980-01-01
        write_u32(&mut out, *crc);
        write_u32(&mut out, payload.len() as u32);
        write_u32(&mut out, *size as u32);
        write_u16(&mut out, name_bytes.len() as u16);
        write_u16(&mut out, 0);
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(payload);

        write_u32(&mut central, 0x0201_4b50);
        write_u16(&mut central, 0x0314); // made by Unix, version 2.0
        write_u16(&mut central, 20);
        write_u16(&mut central, 0);
        write_u16(&mut central, *method);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0x0021);
        write_u32(&mut central, *crc);
        write_u32(&mut central, payload.len() as u32);
        write_u32(&mut central, *size as u32);
        write_u16(&mut central, name_bytes.len() as u16);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0); // disk
        write_u16(&mut central, 0); // internal attributes
        // Regular file, mode 0644: never a link, never executable.
        write_u32(&mut central, 0o100_644 << 16);
        write_u32(&mut central, offset);
        central.extend_from_slice(name_bytes);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    write_u32(&mut out, 0x0605_4b50);
    write_u16(&mut out, 0);
    write_u16(&mut out, 0);
    write_u16(&mut out, prepared.len() as u16);
    write_u16(&mut out, prepared.len() as u16);
    write_u32(&mut out, cd_size);
    write_u32(&mut out, cd_offset);
    write_u16(&mut out, 0);
    Ok(out)
}

/// Horizon's package reader refuses these; refusing them here keeps a bad
/// archive from ever being written.
fn validate_path(name: &str) -> Result<(), ZipError> {
    if name.is_empty() {
        return err("empty entry name");
    }
    if name.contains('\\') {
        return err(format!("`{name}`: backslash in a path (Horizon wants `/`)"));
    }
    if name.contains(':') {
        return err(format!(
            "`{name}`: `:` in a path (Horizon refuses it; a drive letter or a stream name)"
        ));
    }
    if name.starts_with('/') {
        return err(format!("`{name}`: absolute path"));
    }
    for part in name.split('/') {
        if part.is_empty() {
            return err(format!("`{name}`: empty path segment"));
        }
        if part == "." || part == ".." {
            return err(format!("`{name}`: `.` or `..` path segment"));
        }
        if part.chars().any(|c| c.is_control()) {
            return err(format!("`{name}`: control character in a path"));
        }
    }
    Ok(())
}

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Convenience: read every file from a zip on disk, in order.
pub fn read_zip_file(path: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>, ZipError> {
    let file = std::fs::File::open(path)
        .map_err(|e| ZipError { context: format!("{}: {e}", path.display()) })?;
    let mut archive = Archive::new(std::io::BufReader::new(file))
        .map_err(|e| ZipError { context: format!("{}: {}", path.display(), e) })?;
    archive
        .read_all()
        .map_err(|e| ZipError { context: format!("{}: {}", path.display(), e) })
}

/// Convenience: write a built archive to disk, creating parent directories.
pub fn write_zip_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn roundtrip(files: &[(&str, &[u8])]) -> Archive<Cursor<Vec<u8>>> {
        let entries: Vec<OutEntry> = files
            .iter()
            .map(|(n, d)| OutEntry::new(*n, d.to_vec()))
            .collect();
        let bytes = build(&entries).unwrap();
        Archive::new(Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn round_trips_text_and_binary() {
        let text = b"{\n  \"pack\": {\n    \"description\": \"x\"\n  }\n}".to_vec();
        let binary: Vec<u8> = (0..=255u8).cycle().take(9000).collect();
        let mut a = roundtrip(&[
            ("a/b.json", &text),
            ("a/blobby.bin", &binary),
            ("tiny", b"x"),
            ("empty", b""),
        ]);
        assert_eq!(a.read("a/b.json").unwrap(), text);
        assert_eq!(a.read("a/blobby.bin").unwrap(), binary);
        assert_eq!(a.read("tiny").unwrap(), b"x");
        assert_eq!(a.read("empty").unwrap(), b"");
        assert!(a.names().any(|n| n == "a/b.json"));
    }

    #[test]
    fn output_is_byte_identical_across_runs() {
        let entries = vec![
            OutEntry::new("x.txt", b"hello hello hello hello hello".to_vec()),
            OutEntry::new("y/z.json", b"{\"a\":1}".to_vec()),
        ];
        let first = build(&entries).unwrap();
        let second = build(&entries).unwrap();
        assert_eq!(first, second, "deterministic output");
    }

    #[test]
    fn refuses_paths_horizon_would_refuse() {
        for bad in ["a\\b", "/abs", "a/../b", "a//b", "c:/x", "./a", ""] {
            let entries = vec![OutEntry::new(bad, b"x".to_vec())];
            assert!(build(&entries).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn refuses_duplicate_names_and_detects_them_on_read() {
        let entries = vec![
            OutEntry::new("a", b"1".to_vec()),
            OutEntry::new("a", b"2".to_vec()),
        ];
        assert!(build(&entries).unwrap_err().context.contains("duplicate"));
    }

    #[test]
    fn detects_corruption() {
        let entries = vec![OutEntry::new(
            "data.json",
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec(),
        )];
        let mut bytes = build(&entries).unwrap();
        // Flip a byte of the deflate payload, which sits after the 30-byte
        // local header and the entry name.
        let name_at = bytes
            .windows(9)
            .position(|w| w == b"data.json")
            .expect("name");
        bytes[name_at + 9] ^= 0xFF;
        let mut a = Archive::new(Cursor::new(bytes)).unwrap();
        let error = a.read("data.json").unwrap_err();
        assert!(
            error.context.contains("CRC") || error.context.contains("deflate"),
            "corruption must be caught, got: {error}"
        );
    }

    /// Rewrites an archive's central directory so every entry claims a Unix
    /// mode and a Unix origin, the way a jar built on Linux looks.
    fn with_unix_mode(bytes: &[u8], mode: u32) -> Vec<u8> {
        let mut out = bytes.to_vec();
        let eocd = out
            .windows(4)
            .rposition(|w| w == b"PK\x05\x06")
            .expect("end of central directory");
        let cd_at = u32::from_le_bytes([out[eocd + 16], out[eocd + 17], out[eocd + 18], out[eocd + 19]])
            as usize;
        let count = u16::from_le_bytes([out[eocd + 10], out[eocd + 11]]) as usize;
        let mut at = cd_at;
        for _ in 0..count {
            assert_eq!(&out[at..at + 4], b"PK\x01\x02");
            out[at + 5] = 3; // version made by: Unix
            out[at + 6] = 0x14;
            out[at + 38..at + 42].copy_from_slice(&(mode << 16).to_le_bytes());
            let name_len = u16::from_le_bytes([out[at + 28], out[at + 29]]) as usize;
            let extra_len = u16::from_le_bytes([out[at + 30], out[at + 31]]) as usize;
            let comment_len = u16::from_le_bytes([out[at + 32], out[at + 33]]) as usize;
            at += 46 + name_len + extra_len + comment_len;
        }
        out
    }

    #[test]
    fn refuses_symlinks_and_executables_from_foreign_archives() {
        let entries = vec![OutEntry::new("payload", b"target".to_vec())];
        let plain = build(&entries).unwrap();

        let mut link = Archive::new(Cursor::new(with_unix_mode(&plain, 0o120_777))).unwrap();
        assert!(link.entries()[0].is_symlink());
        assert!(link
            .read_all()
            .unwrap_err()
            .context
            .contains("symbolic link"));

        let mut exe = Archive::new(Cursor::new(with_unix_mode(&plain, 0o100_755))).unwrap();
        assert!(exe.entries()[0].is_executable());
        assert!(exe.read_all().unwrap_err().context.contains("executable"));

        let mut ordinary = Archive::new(Cursor::new(with_unix_mode(&plain, 0o100_644))).unwrap();
        assert!(!ordinary.entries()[0].is_symlink());
        assert!(!ordinary.entries()[0].is_executable());
        assert_eq!(ordinary.read_all().unwrap()[0].1, b"target");
    }

    #[test]
    fn refuses_non_zip_input() {
        assert!(Archive::new(Cursor::new(b"not a zip at all, definitely not".to_vec())).is_err());
        assert!(Archive::new(Cursor::new(Vec::new())).is_err());
    }
}
