use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use jiff::tz::TimeZone as JiffTimeZone;
use memchr::{memchr, memmem};

use turbotokens_adapter_common::read_files_parallel;
use turbotokens_core::{PricingMap, format_date_tz, log_level, parse_tz, utc_now};

use crate::{
    cli::{CostMode, SharedArgs},
    daily::{
        DailyDedupOutcome, DailyLoadedEntry, finish_daily_raw_entry, push_deduped_daily_entry,
        scan_daily_line,
    },
    fast::{FxHashMap, SmallIndexVec},
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
}

impl FileCursor {
    fn position(&self) -> u64 {
        self.offset + self.tail.len() as u64
    }
}

/// Outcome of one accepted usage line, with the file's session id attached so
/// consumers can maintain their own aggregations.
pub(crate) enum WatchOutcome {
    Added {
        index: usize,
        session_id: Arc<str>,
    },
    Replaced {
        index: usize,
        previous: Box<DailyLoadedEntry>,
        session_id: Arc<str>,
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
    pub(crate) cursors: FxHashMap<PathBuf, FileCursor>,
    deduped_indexes: FxHashMap<u64, SmallIndexVec>,
    pub(crate) deduped: Vec<DailyLoadedEntry>,
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
            cursors: FxHashMap::default(),
            deduped_indexes: FxHashMap::default(),
            deduped: Vec::new(),
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
        let contents = read_files_parallel(&files, single_thread, |file| {
            fs::read(file).unwrap_or_default()
        });
        for (file, bytes) in files.iter().zip(contents) {
            self.feed_bytes(file, &bytes, sink);
        }
        files
    }

    /// Rescan the data directories: new session files are read whole, appended
    /// bytes are fed incrementally, shrunk files are rescanned from zero.
    pub(crate) fn poll_paths(&mut self, paths: &[PathBuf], sink: &mut impl FnMut(WatchOutcome)) {
        for file in usage_files(paths, None) {
            let Ok(metadata) = fs::metadata(&file) else {
                continue;
            };
            self.poll_file(&file, metadata.len(), sink);
        }
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
        let mut raw_entries = Vec::new();
        let (project, session_id) = {
            let marker = &self.usage_marker;
            let cursor = self
                .cursors
                .entry(path.to_path_buf())
                .or_insert_with(|| file_cursor(path));
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
                match push_deduped_daily_entry(entry, &mut self.deduped_indexes, &mut self.deduped)
                {
                    DailyDedupOutcome::Added(index) => sink(WatchOutcome::Added {
                        index,
                        session_id: Arc::clone(&session_id),
                    }),
                    DailyDedupOutcome::Replaced { index, previous } => {
                        sink(WatchOutcome::Replaced {
                            index,
                            previous,
                            session_id: Arc::clone(&session_id),
                        });
                    }
                    DailyDedupOutcome::Duplicate => {}
                }
            }
        }
    }

    pub(crate) fn poll_file(
        &mut self,
        path: &Path,
        size: u64,
        sink: &mut impl FnMut(WatchOutcome),
    ) {
        let position = self.cursors.get(path).map(FileCursor::position);
        match position {
            // New session file: scan it from the start.
            None => {
                if let Ok(bytes) = fs::read(path) {
                    self.feed_bytes(path, &bytes, sink);
                }
            }
            // Shrunk (rotated or rewritten): reset and rescan from offset 0.
            Some(position) if size < position => {
                self.cursors.remove(path);
                if let Ok(bytes) = fs::read(path) {
                    self.feed_bytes(path, &bytes, sink);
                }
            }
            Some(position) if size > position => {
                if let Some(bytes) = read_appended(path, position) {
                    self.feed_bytes(path, &bytes, sink);
                }
            }
            _ => {}
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
    }
}

fn read_appended(path: &Path, position: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(position)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}
