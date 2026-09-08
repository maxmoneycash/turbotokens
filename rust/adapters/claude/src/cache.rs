//! On-disk parse cache for Claude JSONL usage files.
//!
//! Parsing large JSONL logs dominates report runtime, so each source file gets
//! a sibling cache file holding the timezone/mode/pricing-independent records
//! scanned from its bytes. Repeat runs validate the cache against the file's
//! metadata stamp and rescan changed files. All cache I/O is
//! best-effort: any decode or I/O problem falls back to a full rescan and never
//! fails the report.

use std::{
    fs,
    hash::{Hash, Hasher},
    io::Write as _,
    path::{Path, PathBuf},
};

use rustc_hash::FxHasher;
use turbotokens_core::cache_dir::{CacheRoot, cache_root_from_env, create_cache_dir};

use crate::{CacheCreationRaw, Speed, TokenUsageRaw};

const MAGIC: &[u8; 4] = b"CCPC";
// Version 3 replaces the mtime-only stamp with file identity/change metadata.
// This also invalidates earlier caches that could omit whitespace-formatted JSON.
const VERSION: u32 = 3;

const REPORT_MAGIC: &[u8; 4] = b"CCRB";
const REPORT_VERSION: u32 = 2;
const REPORT_SLOT_COUNT: u64 = 256;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Result of scanning a byte slice for usage records.
///
/// Only complete (newline-terminated) lines feed `entries`/`min_timestamp_ms`;
/// `consumed` marks the offset just past the last newline so a partial trailing
/// line is re-read on the next run. A trailing unterminated line is still
/// scanned into `tail_*` so reports match the historical uncached behavior,
/// but it is never persisted to the cache.
pub(crate) struct ScanResult<R> {
    pub consumed: u64,
    pub min_timestamp_ms: Option<i64>,
    pub entries: Vec<R>,
    pub tail_min_timestamp_ms: Option<i64>,
    pub tail_entries: Vec<R>,
}

impl<R> ScanResult<R> {
    pub fn new() -> Self {
        Self {
            consumed: 0,
            min_timestamp_ms: None,
            entries: Vec::new(),
            tail_min_timestamp_ms: None,
            tail_entries: Vec::new(),
        }
    }
}

impl<R> Default for ScanResult<R> {
    fn default() -> Self {
        Self::new()
    }
}

fn merge_min(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

fn cache_file_path(root: &Path, kind: &str, source: &Path) -> PathBuf {
    let mut hasher = FxHasher::default();
    hasher.write(source.as_os_str().as_encoded_bytes());
    let hash = hasher.finish();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(kind.len() + 22);
    name.push_str(kind);
    name.push('-');
    for shift in (0..16).rev() {
        name.push(HEX[((hash >> (shift * 4)) & 0xf) as usize] as char);
    }
    name.push_str(".bin");
    root.join("parse-v1").join("claude").join(name)
}

/// Scan `path` into raw records, using the on-disk cache when possible.
///
/// Returns the minimum timestamp across all parsed lines and the raw records
/// in file order. `scan` turns bytes into records, `write_entry`/`read_entry`
/// encode and decode a single record for the cache file.
pub(crate) fn cached_scan<R>(
    path: &Path,
    kind: &str,
    scan: impl Fn(&[u8]) -> ScanResult<R>,
    write_entry: impl Fn(&mut Writer, &R),
    read_entry: impl Fn(&mut Reader<'_>) -> Option<R>,
) -> (Option<i64>, Vec<R>) {
    let CacheRoot::Dir(root) = cache_root_from_env() else {
        return scan_uncached(path, &scan);
    };
    let cache_path = cache_file_path(&root, kind, path);
    cached_scan_with_cache_path(path, &cache_path, &scan, &write_entry, &read_entry)
}

fn scan_uncached<R>(path: &Path, scan: &impl Fn(&[u8]) -> ScanResult<R>) -> (Option<i64>, Vec<R>) {
    let Some(result) = with_file_bytes(path, |content| finish_scan(scan(content))) else {
        return (None, Vec::new());
    };
    result
}

/// Read owned bytes before parsing. Session logs can be truncated or rewritten
/// by another process; borrowed memory maps can fault when their backing file
/// shrinks during a scan. Read failures retain the existing empty-result policy.
fn with_file_bytes<R>(path: &Path, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
    let bytes = fs::read(path).ok()?;
    Some(f(&bytes))
}

fn finish_scan<R>(scanned: ScanResult<R>) -> (Option<i64>, Vec<R>) {
    let min_timestamp_ms = merge_min(scanned.min_timestamp_ms, scanned.tail_min_timestamp_ms);
    let mut entries = scanned.entries;
    entries.extend(scanned.tail_entries);
    (min_timestamp_ms, entries)
}

fn cached_scan_with_cache_path<R>(
    path: &Path,
    cache_path: &Path,
    scan: &impl Fn(&[u8]) -> ScanResult<R>,
    write_entry: &impl Fn(&mut Writer, &R),
    read_entry: &impl Fn(&mut Reader<'_>) -> Option<R>,
) -> (Option<i64>, Vec<R>) {
    let Ok(metadata) = fs::metadata(path) else {
        return scan_uncached(path, scan);
    };
    let size = metadata.len();
    let Some(stamp) = metadata_stamp(path, &metadata) else {
        return scan_uncached(path, scan);
    };

    if let Some(cached) = read_cache_file(cache_path, read_entry)
        && size == cached.parsed_len
        && stamp == cached.stamp
    {
        return (cached.min_timestamp_ms, cached.entries);
    }

    // Growth alone cannot distinguish an append from a rewrite or file
    // replacement. Rescan any changed file rather than reuse stale rows.
    let Some(scanned) = with_file_bytes(path, |content| scan(content)) else {
        return (None, Vec::new());
    };
    write_cache_file(
        cache_path,
        &encode_cache(
            scanned.consumed,
            stamp,
            scanned.min_timestamp_ms,
            &scanned.entries,
            write_entry,
        ),
    );
    finish_scan(scanned)
}

/// Shared by parse and whole-report caches. Reading a file can change atime, so
/// only metadata that describes identity or writes belongs in this stamp.
pub(crate) fn metadata_stamp(_path: &Path, metadata: &fs::Metadata) -> Option<u64> {
    let mut hasher = FxHasher::default();
    metadata.len().hash(&mut hasher);
    metadata.modified().ok()?.hash(&mut hasher);
    metadata.created().ok().hash(&mut hasher);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.dev().hash(&mut hasher);
        metadata.ino().hash(&mut hasher);
        metadata.ctime().hash(&mut hasher);
        metadata.ctime_nsec().hash(&mut hasher);
    }
    #[cfg(windows)]
    crate::windows_change_time::change_time(_path)?.hash(&mut hasher);
    Some(hasher.finish())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

struct CachedFile<R> {
    parsed_len: u64,
    stamp: u64,
    min_timestamp_ms: Option<i64>,
    entries: Vec<R>,
}

fn encode_cache<R>(
    parsed_len: u64,
    stamp: u64,
    min_timestamp_ms: Option<i64>,
    entries: &[R],
    write_entry: &impl Fn(&mut Writer, &R),
) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.push_bytes(MAGIC);
    writer.push_u32(VERSION);
    writer.push_u64(parsed_len);
    writer.push_u64(stamp);
    writer.push_i64(min_timestamp_ms.unwrap_or(i64::MIN));
    writer.push_u32(entries.len() as u32);
    for entry in entries {
        write_entry(&mut writer, entry);
    }
    let mut bytes = writer.into_vec();
    let checksum = fnv1a(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    bytes
}

fn decode_cache<R>(
    bytes: &[u8],
    read_entry: &impl Fn(&mut Reader<'_>) -> Option<R>,
) -> Option<CachedFile<R>> {
    let (payload, checksum) = bytes.split_at(bytes.len().checked_sub(8)?);
    let expected = u64::from_le_bytes(checksum.try_into().ok()?);
    if fnv1a(payload) != expected {
        return None;
    }
    let mut reader = Reader::new(payload);
    if reader.read_bytes(4)? != MAGIC {
        return None;
    }
    if reader.read_u32()? != VERSION {
        return None;
    }
    let parsed_len = reader.read_u64()?;
    let stamp = reader.read_u64()?;
    let min_timestamp_ms = match reader.read_i64()? {
        i64::MIN => None,
        value => Some(value),
    };
    let entry_count = reader.read_u32()? as usize;
    let mut entries = Vec::with_capacity(entry_count.min(1 << 20));
    for _ in 0..entry_count {
        entries.push(read_entry(&mut reader)?);
    }
    reader.finish()?;
    Some(CachedFile {
        parsed_len,
        stamp,
        min_timestamp_ms,
        entries,
    })
}

fn read_cache_file<R>(
    path: &Path,
    read_entry: &impl Fn(&mut Reader<'_>) -> Option<R>,
) -> Option<CachedFile<R>> {
    let bytes = fs::read(path).ok()?;
    decode_cache(&bytes, read_entry)
}

fn write_cache_file(path: &Path, bytes: &[u8]) {
    let Some(parent) = path.parent() else {
        return;
    };
    if create_cache_dir(parent).is_err() {
        return;
    }
    let tmp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    // A pre-existing file (including a symlink) must never be overwritten.
    // Concurrent writers can safely skip this best-effort cache update.
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let Ok(mut file) = options.open(&tmp_path) else {
        return;
    };
    let result = file.write_all(bytes);
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
        return;
    }
    if fs::rename(&tmp_path, path).is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
}

/// Bounded blob store for derived report data (final summary rows).
/// The caller folds every input that affects the report — dataset fingerprint,
/// args, binary build — into `key`. Each kind keeps at most 256 slots; collisions
/// replace older blobs. The checksum covers the full key as well as the payload,
/// and reads require an exact key match so collisions only cause cache misses.
pub(crate) fn read_report_blob(kind: &str, key: u64) -> Option<Vec<u8>> {
    let CacheRoot::Dir(root) = cache_root_from_env() else {
        return None;
    };
    let bytes = fs::read(report_blob_path(&root, kind, key)).ok()?;
    decode_report_blob(&bytes, key).map(<[u8]>::to_vec)
}

fn decode_report_blob(bytes: &[u8], key: u64) -> Option<&[u8]> {
    let (payload, checksum) = bytes.split_at(bytes.len().checked_sub(8)?);
    let expected = u64::from_le_bytes(checksum.try_into().ok()?);
    if fnv1a(payload) != expected {
        return None;
    }
    let mut reader = Reader::new(payload);
    if reader.read_bytes(4)? != REPORT_MAGIC
        || reader.read_u32()? != REPORT_VERSION
        || reader.read_u64()? != key
    {
        return None;
    }
    Some(&payload[reader.pos..])
}

pub(crate) fn write_report_blob(kind: &str, key: u64, payload: &[u8]) {
    let CacheRoot::Dir(root) = cache_root_from_env() else {
        return;
    };
    write_cache_file(
        &report_blob_path(&root, kind, key),
        &encode_report_blob(key, payload),
    );
}

fn encode_report_blob(key: u64, payload: &[u8]) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.push_bytes(REPORT_MAGIC);
    writer.push_u32(REPORT_VERSION);
    writer.push_u64(key);
    writer.push_bytes(payload);
    let mut bytes = writer.into_vec();
    bytes.extend_from_slice(&fnv1a(&bytes).to_le_bytes());
    bytes
}

fn report_blob_path(root: &Path, kind: &str, key: u64) -> PathBuf {
    root.join("report-v2")
        .join(format!("{kind}-{:02x}.bin", key % REPORT_SLOT_COUNT))
}

/// Little-endian binary encoder for cache payloads.
pub(crate) struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn push_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn push_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn push_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn push_i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn push_f64(&mut self, value: f64) {
        self.bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    pub fn push_bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn push_str(&mut self, value: &str) {
        self.push_u32(value.len() as u32);
        self.bytes.extend_from_slice(value.as_bytes());
    }

    pub fn push_opt_str(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.push_u8(1);
                self.push_str(value);
            }
            None => self.push_u8(0),
        }
    }

    pub fn push_opt_f64(&mut self, value: Option<f64>) {
        match value {
            Some(value) => {
                self.push_u8(1);
                self.push_f64(value);
            }
            None => self.push_u8(0),
        }
    }

    pub fn push_opt_bool(&mut self, value: Option<bool>) {
        self.push_u8(match value {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        });
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.bytes
    }
}

/// Cursor over a cache payload; every read returns `None` on malformed input
/// so callers can treat any decode problem as a cache miss.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        Some(*self.read_bytes(1)?.first()?)
    }

    pub fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.read_bytes(4)?.try_into().ok()?))
    }

    pub fn read_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.read_bytes(8)?.try_into().ok()?))
    }

    pub fn read_i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.read_bytes(8)?.try_into().ok()?))
    }

    pub fn read_f64(&mut self) -> Option<f64> {
        Some(f64::from_bits(u64::from_le_bytes(
            self.read_bytes(8)?.try_into().ok()?,
        )))
    }

    pub fn read_str(&mut self) -> Option<String> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_bytes(len)?;
        String::from_utf8(bytes.to_vec()).ok()
    }

    pub fn read_opt_str(&mut self) -> Option<Option<String>> {
        match self.read_u8()? {
            0 => Some(None),
            1 => self.read_str().map(Some),
            _ => None,
        }
    }

    pub fn read_opt_f64(&mut self) -> Option<Option<f64>> {
        match self.read_u8()? {
            0 => Some(None),
            1 => self.read_f64().map(Some),
            _ => None,
        }
    }

    pub fn read_opt_bool(&mut self) -> Option<Option<bool>> {
        match self.read_u8()? {
            0 => Some(None),
            1 => Some(Some(false)),
            2 => Some(Some(true)),
            _ => None,
        }
    }

    pub fn finish(self) -> Option<()> {
        (self.pos == self.bytes.len()).then_some(())
    }
}

pub(crate) fn write_token_usage(writer: &mut Writer, usage: TokenUsageRaw) {
    writer.push_u64(usage.input_tokens);
    writer.push_u64(usage.output_tokens);
    writer.push_u64(usage.cache_creation_input_tokens);
    writer.push_u64(usage.cache_read_input_tokens);
    writer.push_u8(match usage.speed {
        None => 0,
        Some(Speed::Standard) => 1,
        Some(Speed::Fast) => 2,
    });
    match &usage.cache_creation {
        Some(cache_creation) => {
            writer.push_u8(1);
            writer.push_u64(cache_creation.ephemeral_5m_input_tokens);
            writer.push_u64(cache_creation.ephemeral_1h_input_tokens);
        }
        None => writer.push_u8(0),
    }
}

pub(crate) fn read_token_usage(reader: &mut Reader<'_>) -> Option<TokenUsageRaw> {
    let input_tokens = reader.read_u64()?;
    let output_tokens = reader.read_u64()?;
    let cache_creation_input_tokens = reader.read_u64()?;
    let cache_read_input_tokens = reader.read_u64()?;
    let speed = match reader.read_u8()? {
        0 => None,
        1 => Some(Speed::Standard),
        2 => Some(Speed::Fast),
        _ => return None,
    };
    let cache_creation = match reader.read_u8()? {
        0 => None,
        1 => Some(CacheCreationRaw {
            ephemeral_5m_input_tokens: reader.read_u64()?,
            ephemeral_1h_input_tokens: reader.read_u64()?,
        }),
        _ => return None,
    };
    Some(TokenUsageRaw {
        input_tokens,
        output_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
        speed,
        cache_creation,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use turbotokens_test_support::fs_fixture;

    use super::{
        Reader, ScanResult, Writer, cache_file_path, cached_scan_with_cache_path, decode_cache,
        encode_cache, read_token_usage, write_token_usage,
    };
    use crate::TokenUsageRaw;

    fn line_scan(bytes: &[u8]) -> ScanResult<String> {
        let mut result = ScanResult::new();
        let mut offset = 0;
        while let Some(newline) = memchr::memchr(b'\n', &bytes[offset..]) {
            result
                .entries
                .push(String::from_utf8_lossy(&bytes[offset..offset + newline]).into_owned());
            offset += newline + 1;
        }
        if offset < bytes.len() {
            result
                .tail_entries
                .push(String::from_utf8_lossy(&bytes[offset..]).into_owned());
        }
        result.consumed = offset as u64;
        result
    }

    #[allow(clippy::ptr_arg)] // matches the generic writer signature over R = String
    fn write_line_entry(writer: &mut Writer, entry: &String) {
        writer.push_str(entry);
    }

    fn read_line_entry(reader: &mut Reader<'_>) -> Option<String> {
        reader.read_str()
    }

    #[test]
    fn scanned_bytes_survive_truncating_the_source_file() {
        let contents = "record\n".repeat(16_384);
        let fixture = fs_fixture!({ "log.jsonl": contents.clone() });
        let path = fixture.path("log.jsonl");
        let scanned = super::with_file_bytes(&path, |bytes| {
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(0)
                .unwrap();
            assert_eq!(bytes, contents.as_bytes());
            line_scan(bytes).entries.len()
        });
        assert_eq!(scanned, Some(16_384));
        assert_eq!(fs::metadata(path).unwrap().len(), 0);
    }

    #[test]
    fn roundtrips_cache_payload() {
        let entries = vec!["alpha".to_string(), "bêtà-日本語".to_string()];
        let bytes = encode_cache(42, 123_456_789, Some(99), &entries, &write_line_entry);

        let decoded = decode_cache(&bytes, &read_line_entry).expect("payload decodes");

        assert_eq!(decoded.parsed_len, 42);
        assert_eq!(decoded.stamp, 123_456_789);
        assert_eq!(decoded.min_timestamp_ms, Some(99));
        assert_eq!(decoded.entries, entries);
    }

    #[test]
    fn rejects_tampered_cache_payload() {
        let entries = vec!["alpha".to_string()];
        let mut bytes = encode_cache(42, 1, None, &entries, &write_line_entry);
        let payload_len = bytes.len() - 8;
        bytes[payload_len - 1] ^= 0xff;

        assert!(decode_cache(&bytes, &read_line_entry).is_none());
        assert!(decode_cache(&bytes[..4], &read_line_entry).is_none());
    }

    #[test]
    fn report_slots_replace_collisions_without_returning_another_key() {
        let fixture = fs_fixture!({ "cache/.keep": "" });
        let root = fixture.path("cache");
        let first_key = 17;
        let second_key = first_key + super::REPORT_SLOT_COUNT;
        let path = super::report_blob_path(&root, "claude-daily", first_key);
        assert_eq!(
            path,
            super::report_blob_path(&root, "claude-daily", second_key)
        );
        assert_ne!(
            path,
            super::report_blob_path(&root, "another-kind", first_key)
        );

        super::write_cache_file(&path, &super::encode_report_blob(first_key, b"first"));
        let first = fs::read(&path).unwrap();
        assert_eq!(
            super::decode_report_blob(&first, first_key),
            Some(&b"first"[..])
        );
        assert!(super::decode_report_blob(&first, second_key).is_none());

        super::write_cache_file(&path, &super::encode_report_blob(second_key, b"second"));
        let second = fs::read(&path).unwrap();
        assert!(super::decode_report_blob(&second, first_key).is_none());
        assert_eq!(
            super::decode_report_blob(&second, second_key),
            Some(&b"second"[..])
        );
    }

    #[test]
    fn report_blobs_reject_corrupt_keys_headers_and_payloads() {
        let key = 17;
        let encoded = super::encode_report_blob(key, b"report rows");
        for len in 0..encoded.len() {
            assert!(super::decode_report_blob(&encoded[..len], key).is_none());
        }
        // Cover magic, version, full key, payload and checksum damage.
        for offset in [0, 4, 8, 16, encoded.len() - 1] {
            let mut corrupt = encoded.clone();
            corrupt[offset] ^= 1;
            assert!(super::decode_report_blob(&corrupt, key).is_none());
        }
        // Even internally checksummed blobs cannot bypass header/key checks.
        for offset in [0, 4, 8] {
            let mut corrupt = encoded.clone();
            corrupt[offset] ^= 1;
            let checksum_offset = corrupt.len() - 8;
            let checksum = super::fnv1a(&corrupt[..checksum_offset]);
            corrupt[checksum_offset..].copy_from_slice(&checksum.to_le_bytes());
            assert!(super::decode_report_blob(&corrupt, key).is_none());
        }
        // The old payload-only format is not accepted in the new directory.
        let mut legacy = b"report rows".to_vec();
        legacy.extend_from_slice(&super::fnv1a(&legacy).to_le_bytes());
        assert!(super::decode_report_blob(&legacy, key).is_none());
    }

    #[test]
    fn report_cache_file_count_is_bounded_across_many_keys() {
        let fixture = fs_fixture!({ "cache/report-v1/legacy.bin": "leave untouched" });
        let root = fixture.path("cache");
        let kinds = ["claude-daily", "another-kind"];
        let key_count = super::REPORT_SLOT_COUNT * 4;
        for kind in kinds {
            for key in 0..key_count {
                let path = super::report_blob_path(&root, kind, key);
                super::write_cache_file(&path, &super::encode_report_blob(key, &key.to_le_bytes()));
                let bytes = fs::read(path).unwrap();
                assert_eq!(
                    super::decode_report_blob(&bytes, key),
                    Some(&key.to_le_bytes()[..])
                );
            }
        }
        let files = fs::read_dir(root.join("report-v2"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(files.len(), kinds.len() * super::REPORT_SLOT_COUNT as usize);
        assert!(
            files
                .iter()
                .all(|file| file.path().extension().unwrap() == "bin")
        );
        for kind in kinds {
            for key in key_count - super::REPORT_SLOT_COUNT..key_count {
                let bytes = fs::read(super::report_blob_path(&root, kind, key)).unwrap();
                assert_eq!(
                    super::decode_report_blob(&bytes, key),
                    Some(&key.to_le_bytes()[..])
                );
                assert!(
                    super::decode_report_blob(&bytes, key - super::REPORT_SLOT_COUNT).is_none()
                );
            }
        }
        assert_eq!(
            fs::read(root.join("report-v1/legacy.bin")).unwrap(),
            b"leave untouched"
        );
    }

    #[test]
    fn rescans_cache_created_before_whitespace_acceptance() {
        let fixture = fs_fixture!({
            "cache/.keep": "",
            "data/log.jsonl": "one\n",
        });
        let path = fixture.path("data/log.jsonl");
        let cache_path = cache_file_path(&fixture.path("cache"), "test", &path);
        let metadata = fs::metadata(&path).unwrap();
        // Version 1 could cache no entries for a valid whitespace-formatted
        // record. Matching size/mtime must not preserve that missing usage.
        let mut old = encode_cache(
            metadata.len(),
            super::metadata_stamp(&path, &metadata).unwrap(),
            None,
            &[],
            &write_line_entry,
        );
        old[4..8].copy_from_slice(&1_u32.to_le_bytes());
        let checksum_offset = old.len() - 8;
        let checksum = super::fnv1a(&old[..checksum_offset]);
        old[checksum_offset..].copy_from_slice(&checksum.to_le_bytes());
        super::write_cache_file(&cache_path, &old);

        let result = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &line_scan,
            &write_line_entry,
            &read_line_entry,
        );

        assert_eq!(result.1, ["one"]);
    }

    #[test]
    fn replacing_a_file_with_matching_size_and_mtime_invalidates_parse_cache() {
        let fixture = fs_fixture!({ "cache/.keep": "", "log.jsonl": "old\n" });
        let path = fixture.path("log.jsonl");
        let fixed_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let set_time = |path: &std::path::Path| {
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(fixed_time))
                .unwrap();
        };
        set_time(&path);
        let cache_path = cache_file_path(&fixture.path("cache"), "test", &path);
        let scan = || {
            cached_scan_with_cache_path(
                &path,
                &cache_path,
                &line_scan,
                &write_line_entry,
                &read_line_entry,
            )
            .1
        };
        assert_eq!(scan(), ["old"]);
        let before = fs::metadata(&path).unwrap();
        let replacement = fixture.path("replacement.tmp");
        fs::write(&replacement, "new\n").unwrap();
        set_time(&replacement);
        fs::rename(&replacement, &path).unwrap();
        let after = fs::metadata(&path).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(scan(), ["new"]);
    }

    #[test]
    fn roundtrips_token_usage_with_all_options() {
        let usages = [
            TokenUsageRaw {
                input_tokens: 1,
                output_tokens: 2,
                cache_creation_input_tokens: 3,
                cache_read_input_tokens: 4,
                speed: None,
                cache_creation: None,
            },
            TokenUsageRaw {
                input_tokens: u64::MAX,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                speed: Some(crate::Speed::Fast),
                cache_creation: Some(crate::CacheCreationRaw {
                    ephemeral_5m_input_tokens: 10,
                    ephemeral_1h_input_tokens: 20,
                }),
            },
        ];
        for usage in usages {
            let mut writer = Writer::new();
            write_token_usage(&mut writer, usage);
            let bytes = writer.into_vec();
            let mut reader = Reader::new(&bytes);

            let decoded = read_token_usage(&mut reader).expect("usage decodes");

            assert_eq!(decoded.input_tokens, usage.input_tokens);
            assert_eq!(decoded.output_tokens, usage.output_tokens);
            assert_eq!(
                decoded.cache_creation_input_tokens,
                usage.cache_creation_input_tokens
            );
            assert_eq!(
                decoded.cache_read_input_tokens,
                usage.cache_read_input_tokens
            );
            assert_eq!(
                decoded.cache_creation_token_count(),
                usage.cache_creation_token_count()
            );
            assert!(matches!(
                (decoded.speed, usage.speed),
                (None, None)
                    | (Some(crate::Speed::Standard), Some(crate::Speed::Standard))
                    | (Some(crate::Speed::Fast), Some(crate::Speed::Fast))
            ));
        }
    }

    #[test]
    fn rescans_appended_lines_without_duplicating_tail() {
        let fixture = fs_fixture!({
            "cache/.keep": "",
            "data/log.jsonl": "one\ntwo\n",
        });
        let path = fixture.path("data/log.jsonl");
        let cache_path = cache_file_path(&fixture.path("cache"), "test", &path);

        let first = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &line_scan,
            &write_line_entry,
            &read_line_entry,
        );
        assert_eq!(first.1, ["one", "two"]);

        // Second run is a pure cache hit.
        let second = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &|_| panic!("an unchanged source should be served from cache"),
            &write_line_entry,
            &read_line_entry,
        );
        assert_eq!(second.1, ["one", "two"]);

        use std::io::Write as _;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "three").unwrap();
        write!(file, "four").unwrap();
        drop(file);

        // Changed sources are rescanned; the unterminated tail is reported
        // but not persisted.
        let third = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &line_scan,
            &write_line_entry,
            &read_line_entry,
        );
        assert_eq!(third.1, ["one", "two", "three", "four"]);

        // Completing the tail line adds exactly it to the cached entries.
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file).unwrap();
        drop(file);
        let fourth = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &line_scan,
            &write_line_entry,
            &read_line_entry,
        );
        assert_eq!(fourth.1, ["one", "two", "three", "four"]);

        // A from-scratch scan of the whole file agrees.
        let content = fs::read(&path).unwrap();
        let fresh = super::finish_scan(line_scan(&content));
        assert_eq!(fourth, fresh);
    }

    #[test]
    fn rescans_when_file_shrinks_or_cache_is_corrupt() {
        let fixture = fs_fixture!({
            "cache/.keep": "",
            "data/log.jsonl": "one\ntwo\n",
        });
        let path = fixture.path("data/log.jsonl");
        let cache_path = cache_file_path(&fixture.path("cache"), "test", &path);

        let first = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &line_scan,
            &write_line_entry,
            &read_line_entry,
        );
        assert_eq!(first.1, ["one", "two"]);

        fs::write(&path, "solo\n").unwrap();
        let shrunk = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &line_scan,
            &write_line_entry,
            &read_line_entry,
        );
        assert_eq!(shrunk.1, ["solo"]);

        fs::write(&cache_path, b"garbage").unwrap();
        let corrupt = cached_scan_with_cache_path(
            &path,
            &cache_path,
            &line_scan,
            &write_line_entry,
            &read_line_entry,
        );
        assert_eq!(corrupt.1, ["solo"]);
    }

    #[test]
    fn rescans_when_file_grows_after_rewrite_or_replacement() {
        for replace in [false, true] {
            let fixture = fs_fixture!({
                "cache/.keep": "",
                "data/log.jsonl": "old\n",
            });
            let path = fixture.path("data/log.jsonl");
            let cache_path = cache_file_path(&fixture.path("cache"), "test", &path);
            let first = cached_scan_with_cache_path(
                &path,
                &cache_path,
                &line_scan,
                &write_line_entry,
                &read_line_entry,
            );
            assert_eq!(first.1, ["old"]);

            if replace {
                let replacement = fixture.path("data/replacement.jsonl");
                fs::write(&replacement, "new first\nnew second\n").unwrap();
                fs::rename(replacement, &path).unwrap();
            } else {
                fs::write(&path, "new first\nnew second\n").unwrap();
            }
            let updated = cached_scan_with_cache_path(
                &path,
                &cache_path,
                &line_scan,
                &write_line_entry,
                &read_line_entry,
            );

            assert_eq!(updated.1, ["new first", "new second"], "replace={replace}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn cache_write_does_not_follow_existing_temporary_symlink() {
        let fixture = fs_fixture!({
            "cache/.keep": "",
            "other-file": "keep this content",
        });
        let path = fixture.path("cache/report.bin");
        let temporary = fixture.path(format!("cache/.report.bin.{}.tmp", std::process::id()));
        std::os::unix::fs::symlink(fixture.path("other-file"), &temporary).unwrap();

        super::write_cache_file(&path, b"cached report");

        assert_eq!(
            fs::read(fixture.path("other-file")).unwrap(),
            b"keep this content"
        );
        assert!(
            fs::symlink_metadata(&temporary)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cache_write_creates_private_files() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = fs_fixture!({ "cache/.keep": "" });
        let path = fixture.path("cache/report.bin");

        super::write_cache_file(&path, b"cached report");

        assert_eq!(fs::read(&path).unwrap(), b"cached report");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
