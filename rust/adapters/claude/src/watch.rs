use std::{
    fs,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use jiff::tz::TimeZone as JiffTimeZone;
use memchr::{memchr, memmem};
use rustc_hash::FxHasher;

use turbotokens_adapter_common::read_files_parallel;
use turbotokens_core::{PricingMap, format_date_tz, log_level, parse_tz, utc_now};

use crate::{
    cli::{CostMode, SharedArgs},
    daily::{
        DailyDedupOutcome, DailyLoadedEntry, finish_daily_raw_entry, push_deduped_daily_entry,
        scan_daily_line,
    },
    fast::{FxHashMap, FxHashSet, SmallIndexVec},
    paths::{extract_project, extract_session_parts, usage_files},
};

/// Read position of one watched file: `offset` bytes were consumed as complete
/// lines, `tail` holds the bytes after the last newline seen so far.
#[derive(Debug, Default)]
pub(crate) struct FileCursor {
    pub(crate) offset: u64,
    pub(crate) tail: Vec<u8>,
    project: Arc<str>,
    session_id: Arc<str>,
    stamp: Option<FileStamp>,
    prefix_hash: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(windows)]
    changed: Option<u64>,
}

impl FileStamp {
    fn new(_path: &Path, metadata: &fs::Metadata) -> Self {
        Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            },
            #[cfg(unix)]
            changed: {
                use std::os::unix::fs::MetadataExt;
                (metadata.ctime(), metadata.ctime_nsec())
            },
            #[cfg(windows)]
            changed: crate::windows_change_time::change_time(_path),
        }
    }

    fn can_reuse(&self) -> bool {
        #[cfg(windows)]
        if self.changed.is_none() {
            return false;
        }
        self.modified.is_some()
    }

    fn same_file(&self, other: &Self) -> bool {
        #[cfg(unix)]
        if self.identity != other.identity {
            return false;
        }
        self.created == other.created
    }
}

impl FileCursor {
    fn position(&self) -> u64 {
        self.offset + self.tail.len() as u64
    }
}

/// Outcome of one accepted usage line. The entry snapshot remains stable even
/// when another line in the same feed replaces its deduplicated index.
pub(crate) enum WatchOutcome {
    /// Existing history changed. Consumers rebuild from the current index;
    /// reseeded records are not new stream activity.
    Reset,
    Added {
        index: usize,
        entry: Box<DailyLoadedEntry>,
        session_id: Arc<str>,
        historical: bool,
    },
    Replaced {
        index: usize,
        previous: Box<DailyLoadedEntry>,
        entry: Box<DailyLoadedEntry>,
        session_id: Arc<str>,
        previous_session_id: Arc<str>,
    },
}

/// Incremental index over the JSONL logs: per-file cursors, partial-line
/// tails, append-only reads, and the message/request dedup map. Both the live
/// tail and the resident daemon index build on top of this machinery.
pub(crate) struct WatchIndex {
    tz: Option<JiffTimeZone>,
    mode: CostMode,
    pricing: Option<PricingMap>,
    usage_marker: memmem::Finder<'static>,
    single_thread: bool,
    pub(crate) cursors: FxHashMap<PathBuf, FileCursor>,
    known_paths: FxHashSet<PathBuf>,
    deduped_indexes: FxHashMap<u64, SmallIndexVec>,
    pub(crate) deduped: Vec<DailyLoadedEntry>,
    deduped_sessions: Vec<Arc<str>>,
    // Retain compact occurrence fingerprints across reindexes. A truncate can
    // be observed while the file is empty, then restore old records as appends.
    // Counting occurrences also preserves distinct records without message IDs.
    known_occurrences: FxHashMap<u64, usize>,
    current_occurrences: FxHashMap<u64, usize>,
}

impl WatchIndex {
    pub(crate) fn new(shared: &SharedArgs) -> Self {
        let tz = parse_tz(shared.timezone.as_deref());
        let pricing = if shared.mode == CostMode::Display {
            None
        } else {
            Some(PricingMap::load_with_overrides(
                shared.offline,
                log_level() != Some(0),
                shared.pricing_overrides.iter(),
            ))
        };
        Self {
            tz,
            mode: shared.mode,
            pricing,
            usage_marker: memmem::Finder::new(br#""usage""#),
            single_thread: shared.single_thread,
            cursors: FxHashMap::default(),
            known_paths: FxHashSet::default(),
            deduped_indexes: FxHashMap::default(),
            deduped: Vec::new(),
            deduped_sessions: Vec::new(),
            known_occurrences: FxHashMap::default(),
            current_occurrences: FxHashMap::default(),
        }
    }

    /// Today's date in the configured timezone.
    pub(crate) fn today(&self) -> String {
        format_date_tz(utc_now(), self.tz.as_ref())
    }

    /// Seed from the existing logs: parallel reads feed the same per-chunk
    /// handler the poller uses for appended bytes. Returns the files seen.
    pub(crate) fn seed(
        &mut self,
        paths: &[PathBuf],
        single_thread: bool,
        sink: &mut impl FnMut(WatchOutcome),
    ) -> Vec<PathBuf> {
        let files = usage_files(paths, None);
        self.seed_files(&files, single_thread, sink);
        files
    }

    fn seed_files(
        &mut self,
        files: &[PathBuf],
        single_thread: bool,
        sink: &mut impl FnMut(WatchOutcome),
    ) {
        let contents = read_files_parallel(files, single_thread, read_snapshot);
        for (file, snapshot) in files.iter().zip(contents) {
            if let Some((bytes, stamp)) = snapshot {
                self.feed_snapshot(file, &bytes, 0, stamp, sink);
            }
        }
    }

    /// Unchanged files need only metadata. Verify the consumed prefix before
    /// accepting appends; changed or removed history rebuilds global dedup so
    /// another file's duplicate can become the surviving entry.
    pub(crate) fn poll_paths(&mut self, paths: &[PathBuf], sink: &mut impl FnMut(WatchOutcome)) {
        let files = usage_files(paths, None);
        let present: FxHashSet<_> = files.iter().collect();
        let mut reset = self.cursors.keys().any(|path| !present.contains(path))
            || files
                .iter()
                .any(|path| self.known_paths.contains(path) && !self.cursors.contains_key(path));
        let mut outcomes = Vec::new();
        if !reset {
            for file in &files {
                if self.poll_file(file, &mut |outcome| outcomes.push(outcome)) {
                    reset = true;
                    break;
                }
            }
        }
        if reset {
            let snapshots = read_files_parallel(&files, self.single_thread, read_snapshot);
            if self.rebuild(&files, snapshots) {
                // Discard append outcomes from the previous index generation.
                sink(WatchOutcome::Reset);
                return;
            }
        }
        for outcome in outcomes {
            sink(outcome);
        }
    }

    fn rebuild(&mut self, files: &[PathBuf], snapshots: Vec<Option<FileSnapshot>>) -> bool {
        // Keep the previous contributions until every surviving file has a
        // coherent snapshot. An active writer or read failure retries next poll.
        let Some(snapshots) = snapshots.into_iter().collect::<Option<Vec<_>>>() else {
            return false;
        };
        self.cursors.clear();
        self.deduped_indexes.clear();
        self.deduped.clear();
        self.deduped_sessions.clear();
        self.current_occurrences.clear();
        for (file, (bytes, stamp)) in files.iter().zip(snapshots) {
            self.feed_snapshot(file, &bytes, 0, stamp, &mut |_| {});
        }
        true
    }

    pub(crate) fn entries_with_sessions(
        &self,
    ) -> impl Iterator<Item = (&DailyLoadedEntry, &Arc<str>)> {
        self.deduped.iter().zip(&self.deduped_sessions)
    }

    /// The single bytes → entries path: startup feeds whole files, the poller
    /// feeds appends. Only newline-terminated bytes are scanned; the
    /// unterminated tail is carried into the next feed.
    pub(crate) fn feed_bytes(
        &mut self,
        path: &Path,
        bytes: &[u8],
        sink: &mut impl FnMut(WatchOutcome),
    ) {
        self.known_paths.insert(path.to_path_buf());
        let mut raw_entries = Vec::new();
        let (project, session_id) = {
            let marker = &self.usage_marker;
            let cursor = self
                .cursors
                .entry(path.to_path_buf())
                .or_insert_with(|| file_cursor(path));
            // Raw feeds cannot establish a filesystem snapshot. Production
            // reads restore these fields after feeding the bytes they hashed.
            cursor.stamp = None;
            cursor.prefix_hash = None;
            cursor.tail.extend_from_slice(bytes);
            let mut consumed = 0;
            while let Some(newline) = memchr(b'\n', &cursor.tail[consumed..]) {
                scan_daily_line(
                    &cursor.tail[consumed..consumed + newline],
                    marker,
                    &mut None,
                    &mut raw_entries,
                );
                consumed += newline + 1;
            }
            cursor.tail.drain(..consumed);
            cursor.offset += consumed as u64;
            (Arc::clone(&cursor.project), Arc::clone(&cursor.session_id))
        };

        for raw in &raw_entries {
            let mut loaded = Vec::new();
            finish_daily_raw_entry(
                raw,
                &project,
                self.tz.as_ref(),
                self.mode,
                self.pricing.as_ref(),
                &mut loaded,
            );
            for entry in loaded {
                let identity = usage_identity(&entry, &session_id);
                match push_deduped_daily_entry(entry, &mut self.deduped_indexes, &mut self.deduped)
                {
                    DailyDedupOutcome::Added(index) => {
                        let current = self.current_occurrences.entry(identity).or_default();
                        *current += 1;
                        let known = self.known_occurrences.entry(identity).or_default();
                        let historical = *current <= *known;
                        *known = (*known).max(*current);
                        self.deduped_sessions.push(Arc::clone(&session_id));
                        sink(WatchOutcome::Added {
                            index,
                            entry: Box::new(self.deduped[index].clone()),
                            session_id: Arc::clone(&session_id),
                            historical,
                        });
                    }
                    DailyDedupOutcome::Replaced { index, previous } => {
                        self.current_occurrences.entry(identity).or_insert(1);
                        self.known_occurrences.entry(identity).or_insert(1);
                        let previous_session_id = std::mem::replace(
                            &mut self.deduped_sessions[index],
                            Arc::clone(&session_id),
                        );
                        sink(WatchOutcome::Replaced {
                            index,
                            previous,
                            entry: Box::new(self.deduped[index].clone()),
                            session_id: Arc::clone(&session_id),
                            previous_session_id,
                        });
                    }
                    DailyDedupOutcome::Duplicate => {
                        self.current_occurrences.entry(identity).or_insert(1);
                        self.known_occurrences.entry(identity).or_insert(1);
                    }
                }
            }
        }
    }

    /// Returns true when this change requires rebuilding the whole index.
    fn poll_file(&mut self, path: &Path, sink: &mut impl FnMut(WatchOutcome)) -> bool {
        let Ok(metadata) = fs::metadata(path) else {
            return false;
        };
        let stamp = FileStamp::new(path, &metadata);
        if stamp.can_reuse()
            && self
                .cursors
                .get(path)
                .and_then(|cursor| cursor.stamp.as_ref())
                == Some(&stamp)
        {
            return false;
        }
        let Some((bytes, stamp)) = read_snapshot(path) else {
            return false;
        };
        let position = if let Some(cursor) = self.cursors.get(path) {
            let Ok(position) = usize::try_from(cursor.position()) else {
                return true;
            };
            if bytes.len() < position
                || cursor
                    .stamp
                    .as_ref()
                    .is_none_or(|old| !old.same_file(&stamp))
                || cursor.prefix_hash != Some(hash_bytes(&bytes[..position]))
            {
                return true;
            }
            position
        } else {
            0
        };
        self.feed_snapshot(path, &bytes, position, stamp, sink);
        false
    }

    fn feed_snapshot(
        &mut self,
        path: &Path,
        bytes: &[u8],
        position: usize,
        stamp: FileStamp,
        sink: &mut impl FnMut(WatchOutcome),
    ) {
        let prefix_hash = hash_bytes(bytes);
        self.feed_bytes(path, &bytes[position..], sink);
        if let Some(cursor) = self.cursors.get_mut(path) {
            cursor.stamp = Some(stamp);
            cursor.prefix_hash = Some(prefix_hash);
        }
    }
}

fn file_cursor(path: &Path) -> FileCursor {
    let (session_id, _) = extract_session_parts(path);
    FileCursor {
        offset: 0,
        tail: Vec::new(),
        project: Arc::from(extract_project(path)),
        session_id: Arc::from(session_id),
        stamp: None,
        prefix_hash: None,
    }
}

type FileSnapshot = (Vec<u8>, FileStamp);

fn read_snapshot(path: &Path) -> Option<FileSnapshot> {
    let mut file = fs::File::open(path).ok()?;
    let before = FileStamp::new(path, &file.metadata().ok()?);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let after = FileStamp::new(path, &file.metadata().ok()?);
    // Retry on the next poll if the writer changed the file during this read.
    (before == after && after.size == bytes.len() as u64).then_some((bytes, after))
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = FxHasher::default();
    hasher.write(bytes);
    hasher.finish()
}

fn usage_identity(entry: &DailyLoadedEntry, session_id: &str) -> u64 {
    let mut hasher = FxHasher::default();
    if let Some(message_id) = &entry.message_id {
        0u8.hash(&mut hasher);
        message_id.hash(&mut hasher);
        entry.request_id.hash(&mut hasher);
    } else {
        1u8.hash(&mut hasher);
        entry.timestamp_ms.hash(&mut hasher);
        entry.project.hash(&mut hasher);
        session_id.hash(&mut hasher);
        entry.model.hash(&mut hasher);
        entry.usage.input_tokens.hash(&mut hasher);
        entry.usage.output_tokens.hash(&mut hasher);
        entry.usage.cache_creation_token_count().hash(&mut hasher);
        entry.usage.cache_read_input_tokens.hash(&mut hasher);
        entry.cost.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use turbotokens_test_support::fs_fixture;

    fn shared() -> SharedArgs {
        SharedArgs {
            mode: CostMode::Display,
            timezone: Some("UTC".to_string()),
            ..SharedArgs::default()
        }
    }

    fn line(id: &str, output: u64) -> String {
        serde_json::json!({
            "timestamp": "2026-07-27T18:00:00.000Z", "requestId": format!("req-{id}"),
            "message": {"id": id, "model": "claude-sonnet-4", "usage": {
                "input_tokens": 100, "output_tokens": output,
            }},
        })
        .to_string()
            + "\n"
    }

    #[test]
    fn verifies_partial_line_bytes_before_accepting_an_append() {
        let record = line("msg-1", 20);
        let split = record.len() / 2;
        let fixture = fs_fixture!({
            "projects/proj-a/session.jsonl": &record[..split],
        });
        let paths = [fixture.root().to_path_buf()];
        let mut index = WatchIndex::new(&shared());
        index.seed(&paths, true, &mut |_| {});
        assert!(index.deduped.is_empty());
        let path = fixture.path("projects/proj-a/session.jsonl");
        fs::write(&path, &record).unwrap();
        let mut outcomes = Vec::new();
        index.poll_paths(&paths, &mut |outcome| outcomes.push(outcome));
        assert!(matches!(outcomes.as_slice(), [WatchOutcome::Added { .. }]));
        assert_eq!(index.deduped.len(), 1);
        assert_eq!(index.deduped[0].usage.output_tokens, 20);
        assert!(index.cursors[&path].tail.is_empty());
    }

    #[test]
    fn reset_discards_append_outcomes_from_the_previous_index() {
        let initial = line("msg-1", 20);
        let fixture = fs_fixture!({
            "projects/proj-a/a.jsonl": initial.clone(),
            "projects/proj-a/z.jsonl": line("msg-2", 30),
        });
        let paths = [fixture.root().to_path_buf()];
        let mut index = WatchIndex::new(&shared());
        index.seed(&paths, true, &mut |_| {});
        fs::write(
            fixture.path("projects/proj-a/a.jsonl"),
            format!("{initial}{}", line("msg-3", 40)),
        )
        .unwrap();
        let rewritten = fixture.path("projects/proj-a/z.jsonl");
        fs::write(&rewritten, line("msg-4", 50)).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&rewritten)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            .unwrap();
        let mut outcomes = Vec::new();
        index.poll_paths(&paths, &mut |outcome| outcomes.push(outcome));
        assert!(matches!(outcomes.as_slice(), [WatchOutcome::Reset]));
        assert_eq!(index.deduped.len(), 3);
        assert_eq!(
            index
                .deduped
                .iter()
                .map(|entry| entry.usage.output_tokens)
                .sum::<u64>(),
            110
        );
    }

    #[test]
    fn failed_rebuild_keeps_prior_contributions_until_all_snapshots_are_ready() {
        let fixture = fs_fixture!({
            "projects/proj-a/a.jsonl": line("msg-1", 20),
            "projects/proj-a/b.jsonl": line("msg-2", 30),
        });
        let mut index = WatchIndex::new(&shared());
        let files = index.seed(&[fixture.root().to_path_buf()], true, &mut |_| {});
        let original_hash = index.cursors[&files[0]].prefix_hash;
        fs::write(&files[0], line("msg-3", 80)).unwrap();
        // Model read_snapshot rejecting the second file while its writer is
        // changing it. Installing only the first snapshot would lose usage.
        assert!(!index.rebuild(&files, vec![read_snapshot(&files[0]), None]));
        assert_eq!(index.cursors.len(), 2);
        assert_eq!(index.cursors[&files[0]].prefix_hash, original_hash);
        assert_eq!(
            index
                .deduped
                .iter()
                .map(|entry| entry.usage.output_tokens)
                .sum::<u64>(),
            50
        );
        let snapshots = files.iter().map(|file| read_snapshot(file)).collect();
        assert!(index.rebuild(&files, snapshots));
        assert_eq!(index.deduped.len(), 2);
        assert_eq!(
            index
                .deduped
                .iter()
                .map(|entry| entry.usage.output_tokens)
                .sum::<u64>(),
            110
        );
    }

    #[cfg(unix)]
    #[test]
    fn detects_atomic_replacement_with_preserved_size_and_mtime() {
        let initial = line("msg-1", 20);
        let replacement = line("msg-2", 40);
        assert_eq!(initial.len(), replacement.len());
        let fixture = fs_fixture!({
            "projects/proj-a/session.jsonl": initial,
        });
        let paths = [fixture.root().to_path_buf()];
        let path = fixture.path("projects/proj-a/session.jsonl");
        let original_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let mut index = WatchIndex::new(&shared());
        index.seed(&paths, true, &mut |_| {});
        let staged = fixture.path("replacement.tmp");
        fs::write(&staged, replacement).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&staged)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_mtime))
            .unwrap();
        fs::rename(staged, &path).unwrap();
        let mut outcomes = Vec::new();
        index.poll_paths(&paths, &mut |outcome| outcomes.push(outcome));
        assert!(matches!(outcomes.as_slice(), [WatchOutcome::Reset]));
        assert_eq!(index.deduped.len(), 1);
        assert_eq!(index.deduped[0].usage.output_tokens, 40);
    }

    #[cfg(unix)]
    #[test]
    fn detects_in_place_rewrite_with_preserved_size_and_mtime() {
        use std::os::unix::fs::MetadataExt;

        let initial = line("msg-1", 20);
        let replacement = line("msg-2", 40);
        assert_eq!(initial.len(), replacement.len());
        let fixture = fs_fixture!({
            "projects/proj-a/session.jsonl": initial,
        });
        let paths = [fixture.root().to_path_buf()];
        let path = fixture.path("projects/proj-a/session.jsonl");
        let original = fs::metadata(&path).unwrap();
        let mut index = WatchIndex::new(&shared());
        index.seed(&paths, true, &mut |_| {});
        fs::write(&path, replacement).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original.modified().unwrap()))
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().ino(), original.ino());
        let mut outcomes = Vec::new();
        index.poll_paths(&paths, &mut |outcome| outcomes.push(outcome));
        assert!(matches!(outcomes.as_slice(), [WatchOutcome::Reset]));
        assert_eq!(index.deduped.len(), 1);
        assert_eq!(index.deduped[0].usage.output_tokens, 40);
    }
}
