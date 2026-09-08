use std::{
    fs,
    hash::Hasher,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::UNIX_EPOCH,
};

use jiff::tz::TimeZone as JiffTimeZone;
use memchr::{memchr, memmem};
use rustc_hash::FxHasher;
use serde::Deserialize;

use crate::{
    ModelBreakdown, PricingMap, Result, Speed, TimestampMs, TokenCounts, TokenUsageRaw,
    UsageSummary,
    cli::{CostMode, SharedArgs},
    cost_and_missing_model_for_usage,
    fast::{FxHashMap, SmallIndexVec, suffix_string},
    log_level, parse_ts_timestamp, parse_tz,
};

use super::{
    advisor_usages_from_line, cache, chunk_file_indexes_by_size, has_unsupported_null_field,
    has_usage_object, is_semver_prefix,
    paths::{claude_paths, extract_project, usage_files},
    usage_dedupe_hash,
};

pub(super) fn load_daily_summaries_inner(
    shared: &SharedArgs,
    project_filter: Option<&str>,
    group_by_project: bool,
) -> Result<Vec<UsageSummary>> {
    let paths = claude_paths()?;
    let files = usage_files(&paths, project_filter);
    if files.is_empty() {
        return Ok(Vec::new());
    }

    // Repeat reports over an unchanged dataset are served from the report
    // cache: the key folds in the file list, file metadata stamps, every arg that
    // affects loading, and the binary build (embedded pricing can change
    // between builds). Online mode is excluded because a pricing refresh would
    // make a cached report stale.
    let tz = parse_tz(shared.timezone.as_deref()).unwrap_or_else(JiffTimeZone::system);
    let report_key = daily_report_key(shared, &files, project_filter, group_by_project, &tz);
    if let Some(key) = report_key
        && let Some(summaries) =
            cache::read_report_blob("claude-daily", key).and_then(|bytes| decode_summaries(&bytes))
    {
        return Ok(summaries);
    }

    let pricing = if shared.mode == CostMode::Display {
        None
    } else {
        Some(PricingMap::load_with_overrides(
            shared.offline,
            log_level() != Some(0),
            shared.pricing_overrides.iter(),
        ))
    };
    let mode = shared.mode;
    let loaded_files = if shared.single_thread {
        files
            .iter()
            .map(|file| read_daily_usage_file(file, Some(&tz), mode, pricing.as_ref()))
            .collect::<Vec<_>>()
    } else {
        read_daily_usage_files_parallel(&files, Some(&tz), mode, pricing.as_ref())
    };

    let entry_capacity = loaded_files.iter().map(|file| file.entries.len()).sum();
    let mut deduped_indexes: FxHashMap<u64, SmallIndexVec> =
        FxHashMap::with_capacity_and_hasher(entry_capacity, Default::default());
    let mut deduped = Vec::with_capacity(entry_capacity);
    for loaded_file in loaded_files {
        for entry in loaded_file.entries {
            if let Some(filter) = project_filter
                && entry.project.as_ref() != filter
            {
                continue;
            }
            push_deduped_daily_entry(entry, &mut deduped_indexes, &mut deduped);
        }
    }

    let summaries: Vec<UsageSummary> = if group_by_project {
        // FxHashMap + a final key sort produces the same date/project-ordered
        // summaries as a BTreeMap without cloning the key string per entry.
        let mut groups = FxHashMap::<(Arc<str>, Arc<str>), DailyAccumulator>::default();
        for entry in &deduped {
            let key = (Arc::clone(&entry.date), Arc::clone(&entry.project));
            groups.entry(key).or_default().add_entry(entry);
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by(|(a, _), (b, _)| a.cmp(b));
        groups
            .into_iter()
            .map(|((date, project), group)| {
                let mut summary = group.into_summary();
                summary.date = Some(date.to_string());
                summary.project = Some(project.to_string());
                summary
            })
            .collect()
    } else {
        let mut groups = FxHashMap::<Arc<str>, DailyAccumulator>::default();
        for entry in &deduped {
            if let Some(group) = groups.get_mut(&*entry.date) {
                group.add_entry(entry);
            } else {
                let mut group = DailyAccumulator::default();
                group.add_entry(entry);
                groups.insert(Arc::clone(&entry.date), group);
            }
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by(|(a, _), (b, _)| a.cmp(b));
        groups
            .into_iter()
            .map(|(key, group)| {
                let mut summary = group.into_summary();
                summary.date = Some(key.to_string());
                summary
            })
            .collect()
    };
    if let Some(key) = report_key {
        cache::write_report_blob("claude-daily", key, &encode_summaries(&summaries));
    }
    Ok(summaries)
}

/// Folds everything that can change the daily report into one hash. Returns
/// `None` when the report may depend on network-fetched pricing (online
/// mode), which a cached report cannot track.
fn daily_report_key(
    shared: &SharedArgs,
    files: &[PathBuf],
    project_filter: Option<&str>,
    group_by_project: bool,
    timezone: &JiffTimeZone,
) -> Option<u64> {
    if !shared.offline {
        return None;
    }
    let mut hasher = FxHasher::default();
    hasher.write(b"report-v2");
    // Binary identity: a rebuild can change the embedded pricing snapshot.
    if let Ok(exe) = std::env::current_exe()
        && let Ok(metadata) = exe.metadata()
    {
        hasher.write_u64(metadata.len());
        if let Ok(modified) = metadata.modified()
            && let Ok(since_epoch) = modified.duration_since(UNIX_EPOCH)
        {
            hasher.write_u128(since_epoch.as_nanos());
        }
    }
    // Hash the resolved zone so changing TZ or the system zone invalidates
    // reports even when --timezone is omitted. Opaque local TZif files cannot
    // be identified reliably, so leave their reports uncached.
    let timezone = jiff::fmt::temporal::DateTimePrinter::new()
        .time_zone_to_string(timezone)
        .ok()?;
    hasher.write(timezone.as_bytes());
    hasher.write_u8(match shared.mode {
        CostMode::Auto => 0,
        CostMode::Calculate => 1,
        CostMode::Display => 2,
    });
    hasher.write_u8(u8::from(group_by_project));
    hasher.write(project_filter.unwrap_or_default().as_bytes());
    for (model, override_) in &shared.pricing_overrides {
        hasher.write(model.as_bytes());
        for value in [
            override_.input_cost_per_token,
            override_.output_cost_per_token,
            override_.cache_creation_input_token_cost,
            override_.cache_read_input_token_cost,
            override_.input_cost_per_token_above_200k_tokens,
            override_.output_cost_per_token_above_200k_tokens,
            override_.cache_creation_input_token_cost_above_200k_tokens,
            override_.cache_read_input_token_cost_above_200k_tokens,
            override_.fast_multiplier,
        ] {
            hasher.write_u64(value.map_or(u64::MAX, f64::to_bits));
        }
        hasher.write_u64(override_.max_input_tokens.unwrap_or(u64::MAX));
    }
    // Statting thousands of log files serially dominates a report-cache hit,
    // so metadata is gathered in parallel and folded in file order.
    let metadata = parallel_metadata(files, shared.single_thread);
    for (file, entry) in files.iter().zip(metadata) {
        hasher.write(file.as_os_str().as_encoded_bytes());
        match entry {
            Some((len, stamp)) => {
                hasher.write_u64(len);
                hasher.write_u64(stamp);
            }
            None => return None,
        }
    }
    Some(hasher.finish())
}

fn file_metadata(file: &PathBuf) -> Option<(u64, u64)> {
    let metadata = fs::metadata(file).ok()?;
    Some((metadata.len(), cache::metadata_stamp(&metadata)?))
}

fn parallel_metadata(files: &[PathBuf], single_thread: bool) -> Vec<Option<(u64, u64)>> {
    let worker_count = if single_thread {
        1
    } else {
        thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(files.len())
    };
    if worker_count <= 1 {
        return files.iter().map(file_metadata).collect();
    }
    let chunk_size = files.len().div_ceil(worker_count);
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for chunk in files.chunks(chunk_size) {
            handles.push(scope.spawn(|| chunk.iter().map(file_metadata).collect::<Vec<_>>()));
        }
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("metadata worker panicked"))
            .collect()
    })
}

fn encode_summaries(summaries: &[UsageSummary]) -> Vec<u8> {
    let mut writer = cache::Writer::new();
    writer.push_u32(summaries.len() as u32);
    for summary in summaries {
        writer.push_opt_str(summary.date.as_deref());
        writer.push_opt_str(summary.month.as_deref());
        writer.push_opt_str(summary.week.as_deref());
        writer.push_opt_str(summary.session_id.as_deref());
        writer.push_opt_str(summary.project_path.as_deref());
        writer.push_opt_str(summary.last_activity.as_deref());
        writer.push_opt_str(summary.first_activity.as_deref());
        writer.push_u64(summary.input_tokens);
        writer.push_u64(summary.output_tokens);
        writer.push_u64(summary.cache_creation_tokens);
        writer.push_u64(summary.cache_read_tokens);
        writer.push_u64(summary.extra_total_tokens);
        writer.push_f64(summary.total_cost);
        writer.push_opt_f64(summary.credits);
        match summary.message_count {
            Some(count) => {
                writer.push_u8(1);
                writer.push_u64(count);
            }
            None => writer.push_u8(0),
        }
        writer.push_u32(summary.models_used.len() as u32);
        for model in &summary.models_used {
            writer.push_str(model);
        }
        writer.push_u32(summary.model_breakdowns.len() as u32);
        for breakdown in &summary.model_breakdowns {
            writer.push_str(&breakdown.model_name);
            writer.push_u64(breakdown.input_tokens);
            writer.push_u64(breakdown.output_tokens);
            writer.push_u64(breakdown.cache_creation_tokens);
            writer.push_u64(breakdown.cache_read_tokens);
            writer.push_u64(breakdown.extra_total_tokens);
            writer.push_f64(breakdown.cost);
            writer.push_u8(u8::from(breakdown.missing_pricing));
        }
        writer.push_opt_str(summary.project.as_deref());
        match &summary.versions {
            Some(versions) => {
                writer.push_u8(1);
                writer.push_u32(versions.len() as u32);
                for version in versions {
                    writer.push_str(version);
                }
            }
            None => writer.push_u8(0),
        }
    }
    writer.into_vec()
}

fn decode_summaries(bytes: &[u8]) -> Option<Vec<UsageSummary>> {
    let mut reader = cache::Reader::new(bytes);
    let count = reader.read_u32()? as usize;
    let mut summaries = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let date = reader.read_opt_str()?;
        let month = reader.read_opt_str()?;
        let week = reader.read_opt_str()?;
        let session_id = reader.read_opt_str()?;
        let project_path = reader.read_opt_str()?;
        let last_activity = reader.read_opt_str()?;
        let first_activity = reader.read_opt_str()?;
        let input_tokens = reader.read_u64()?;
        let output_tokens = reader.read_u64()?;
        let cache_creation_tokens = reader.read_u64()?;
        let cache_read_tokens = reader.read_u64()?;
        let extra_total_tokens = reader.read_u64()?;
        let total_cost = reader.read_f64()?;
        let credits = reader.read_opt_f64()?;
        let message_count = match reader.read_u8()? {
            0 => None,
            1 => Some(reader.read_u64()?),
            _ => return None,
        };
        let models_count = reader.read_u32()? as usize;
        let mut models_used = Vec::with_capacity(models_count.min(1 << 10));
        for _ in 0..models_count {
            models_used.push(reader.read_str()?);
        }
        let breakdowns_count = reader.read_u32()? as usize;
        let mut model_breakdowns = Vec::with_capacity(breakdowns_count.min(1 << 10));
        for _ in 0..breakdowns_count {
            model_breakdowns.push(ModelBreakdown {
                model_name: reader.read_str()?,
                input_tokens: reader.read_u64()?,
                output_tokens: reader.read_u64()?,
                cache_creation_tokens: reader.read_u64()?,
                cache_read_tokens: reader.read_u64()?,
                extra_total_tokens: reader.read_u64()?,
                cost: reader.read_f64()?,
                missing_pricing: match reader.read_u8()? {
                    0 => false,
                    1 => true,
                    _ => return None,
                },
            });
        }
        let project = reader.read_opt_str()?;
        let versions = match reader.read_u8()? {
            0 => None,
            1 => {
                let versions_count = reader.read_u32()? as usize;
                let mut versions = Vec::with_capacity(versions_count.min(1 << 10));
                for _ in 0..versions_count {
                    versions.push(reader.read_str()?);
                }
                Some(versions)
            }
            _ => return None,
        };
        summaries.push(UsageSummary {
            date,
            month,
            week,
            session_id,
            project_path,
            last_activity,
            first_activity,
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
            extra_total_tokens,
            total_cost,
            credits,
            message_count,
            models_used,
            model_breakdowns,
            project,
            versions,
        });
    }
    reader.finish()?;
    Some(summaries)
}

#[derive(Debug)]
struct DailyLoadedFile {
    entries: Vec<DailyLoadedEntry>,
}

#[derive(Clone, Debug)]
pub(super) struct DailyLoadedEntry {
    pub(super) timestamp_ms: i64,
    pub(super) date: Arc<str>,
    pub(super) project: Arc<str>,
    pub(super) usage: TokenUsageRaw,
    pub(super) cost: f64,
    pub(super) model: Option<String>,
    pub(super) missing_pricing_model: Option<String>,
    pub(super) message_id: Option<String>,
    pub(super) request_id: Option<String>,
    pub(super) is_sidechain: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyUsageEntry {
    timestamp: String,
    message: DailyUsageMessage,
    version: Option<String>,
    session_id: Option<String>,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
    request_id: Option<String>,
    is_sidechain: Option<bool>,
}

impl From<DailyAgentProgressEntry> for DailyUsageEntry {
    fn from(entry: DailyAgentProgressEntry) -> Self {
        DailyUsageEntry {
            timestamp: entry.data.message.timestamp,
            message: entry.data.message.message,
            version: None,
            session_id: None,
            cost_usd: entry.data.message.cost_usd,
            request_id: entry.data.message.request_id,
            is_sidechain: entry.data.message.is_sidechain,
        }
    }
}

/// Parses one usage line. Matches the acceptance set of the former
/// `#[serde(untagged)]` enum: a direct entry is tried first, an agent-progress
/// wrapper second — but without buffering the whole line into a generic
/// serde value tree, which dominated per-line parse cost.
fn parse_daily_usage_line(line: &[u8]) -> Option<DailyUsageEntry> {
    if let Ok(entry) = serde_json::from_slice::<DailyUsageEntry>(line) {
        return Some(entry);
    }
    serde_json::from_slice::<DailyAgentProgressEntry>(line)
        .ok()
        .map(DailyUsageEntry::from)
}

#[derive(Debug, Deserialize)]
struct DailyAgentProgressEntry {
    data: DailyAgentProgressData,
}

#[derive(Debug, Deserialize)]
struct DailyAgentProgressData {
    message: DailyAgentProgressMessage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyAgentProgressMessage {
    timestamp: String,
    message: DailyUsageMessage,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
    request_id: Option<String>,
    is_sidechain: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DailyUsageMessage {
    usage: TokenUsageRaw,
    model: Option<String>,
    id: Option<String>,
}

fn read_daily_usage_files_parallel(
    files: &[PathBuf],
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
) -> Vec<DailyLoadedFile> {
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(files.len());
    if worker_count <= 1 {
        return files
            .iter()
            .map(|file| read_daily_usage_file(file, tz, mode, pricing))
            .collect();
    }

    let chunks = chunk_file_indexes_by_size(files, worker_count);
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for chunk in chunks {
            let tz = tz.cloned();
            handles.push(scope.spawn(move || {
                chunk
                    .into_iter()
                    .map(|index| {
                        (
                            index,
                            read_daily_usage_file(&files[index], tz.as_ref(), mode, pricing),
                        )
                    })
                    .collect::<Vec<_>>()
            }));
        }
        let mut loaded_files = Vec::with_capacity(files.len());
        loaded_files.resize_with(files.len(), || None);
        for (index, file) in handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("daily usage worker panicked"))
        {
            loaded_files[index] = Some(file);
        }
        loaded_files
            .into_iter()
            .map(|file| file.expect("daily usage worker returned every file"))
            .collect()
    })
}

fn read_daily_usage_file(
    path: &Path,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
) -> DailyLoadedFile {
    let project: Arc<str> = Arc::from(extract_project(path));
    let (_, raw_entries) = cache::cached_scan(
        path,
        "daily",
        scan_daily_bytes,
        write_daily_raw_entry,
        read_daily_raw_entry,
    );
    let mut loaded_file = DailyLoadedFile {
        entries: Vec::with_capacity(raw_entries.len()),
    };
    // Session logs are chronological, so consecutive entries usually share a
    // local date. Interning hands out one allocation per day per file instead
    // of one formatted `String` per entry.
    let mut date_interner = DateInterner::new();
    for raw_entry in raw_entries {
        finish_daily_raw_entry_owned(
            raw_entry,
            &project,
            tz,
            mode,
            pricing,
            &mut date_interner,
            &mut loaded_file.entries,
        );
    }
    loaded_file
}

/// Hands out one shared `Arc<str>` per distinct local date. A tiny ring is
/// enough: entries arrive in (roughly) chronological order per file, so the
/// working set is the current day plus the occasional out-of-order neighbor.
pub(super) struct DateInterner {
    recent: [((i32, u32, u32), Arc<str>); 4],
    len: usize,
}

impl DateInterner {
    pub(super) fn new() -> Self {
        Self {
            recent: std::array::from_fn(|_| ((0, 0, 0), Arc::from(""))),
            len: 0,
        }
    }

    pub(super) fn intern(&mut self, timestamp_ms: i64, tz: Option<&JiffTimeZone>) -> Arc<str> {
        let day = crate::civil_day_tz(TimestampMs::from_millis(timestamp_ms), tz);
        for (cached_day, date) in &self.recent[..self.len] {
            if *cached_day == day {
                return Arc::clone(date);
            }
        }
        let date: Arc<str> = Arc::from(crate::format_civil_day(day.0, day.1, day.2));
        if self.len < self.recent.len() {
            self.recent[self.len] = (day, Arc::clone(&date));
            self.len += 1;
        } else {
            self.recent.rotate_left(1);
            self.recent[self.recent.len() - 1] = (day, Arc::clone(&date));
        }
        date
    }
}

/// Timezone/mode/pricing-independent record scanned from one usage line.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DailyRawEntry {
    pub(super) timestamp_ms: i64,
    pub(super) usage: TokenUsageRaw,
    pub(super) cost_usd: Option<f64>,
    pub(super) model: Option<String>,
    pub(super) message_id: Option<String>,
    pub(super) request_id: Option<String>,
    pub(super) is_sidechain: Option<bool>,
    pub(super) advisors: Vec<(String, TokenUsageRaw)>,
}

fn scan_daily_bytes(bytes: &[u8]) -> cache::ScanResult<DailyRawEntry> {
    let usage_marker = memmem::Finder::new(br#""usage""#);
    let mut result = cache::ScanResult::new();
    // One SIMD pass for the usage marker instead of walking all ~2M lines and
    // probing each: hits map back to their containing line with localized
    // memchr searches. Complete lines (up to the last newline) feed `entries`,
    // the trailing fragment feeds `tail_entries`, exactly as a line walk would.
    let tail_start = memchr::memrchr(b'\n', bytes).map_or(0, |index| index + 1);
    let mut scanned_lines_up_to = 0_usize;
    let mut tail_scanned = false;
    for hit in usage_marker.find_iter(bytes) {
        if hit < tail_start {
            if hit < scanned_lines_up_to {
                continue;
            }
            let line_start = memchr::memrchr(b'\n', &bytes[..hit]).map_or(0, |index| index + 1);
            let line_end = memchr(b'\n', &bytes[hit..]).map_or(bytes.len(), |index| hit + index);
            scan_daily_line(
                &bytes[line_start..line_end],
                &usage_marker,
                &mut result.min_timestamp_ms,
                &mut result.entries,
            );
            scanned_lines_up_to = line_end + 1;
        } else if !tail_scanned {
            tail_scanned = true;
            scan_daily_line(
                &bytes[tail_start..],
                &usage_marker,
                &mut result.tail_min_timestamp_ms,
                &mut result.tail_entries,
            );
        }
    }
    result.consumed = tail_start as u64;
    result
}

pub(super) fn scan_daily_line(
    line: &[u8],
    usage_marker: &memmem::Finder,
    min_timestamp_ms: &mut Option<i64>,
    out: &mut Vec<DailyRawEntry>,
) {
    if !has_usage_object(line, usage_marker) {
        return;
    }
    if has_unsupported_null_field(line) {
        return;
    }
    let Some(data) = parse_daily_usage_line(line) else {
        return;
    };
    let Some(timestamp) = parse_ts_timestamp(&data.timestamp) else {
        return;
    };
    *min_timestamp_ms = Some(
        min_timestamp_ms.map_or(timestamp.as_millis(), |current: i64| {
            current.min(timestamp.as_millis())
        }),
    );
    if !is_valid_daily_usage_entry(&data) {
        return;
    }
    let advisors = advisor_usages_from_line(line)
        .into_iter()
        .map(|advisor| (advisor.model, advisor.usage))
        .collect();
    out.push(DailyRawEntry {
        timestamp_ms: timestamp.as_millis(),
        usage: data.message.usage,
        cost_usd: data.cost_usd,
        model: data.message.model,
        message_id: data.message.id,
        request_id: data.request_id,
        is_sidechain: data.is_sidechain,
        advisors,
    });
}

pub(super) fn finish_daily_raw_entry(
    raw: &DailyRawEntry,
    project: &Arc<str>,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
    out: &mut Vec<DailyLoadedEntry>,
) {
    // Watch/live paths handle a handful of entries per poll, so the clone is
    // irrelevant there; the bulk report path consumes the raw entries and
    // moves their strings into the loaded entries instead.
    let mut interner = DateInterner::new();
    finish_daily_raw_entry_owned(raw.clone(), project, tz, mode, pricing, &mut interner, out);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_daily_raw_entry_owned(
    raw: DailyRawEntry,
    project: &Arc<str>,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
    interner: &mut DateInterner,
    out: &mut Vec<DailyLoadedEntry>,
) {
    let usage = raw.usage;
    let date = interner.intern(raw.timestamp_ms, tz);
    let (cost, missing_pricing_model) =
        cost_and_missing_model_for_usage(raw.model.as_deref(), usage, raw.cost_usd, mode, pricing);
    let model = raw.model.and_then(|model| {
        if model == "<synthetic>" {
            None
        } else if matches!(usage.speed, Some(Speed::Fast)) {
            Some(suffix_string(&model, "-fast"))
        } else {
            Some(model)
        }
    });
    if raw.advisors.is_empty() {
        out.push(DailyLoadedEntry {
            timestamp_ms: raw.timestamp_ms,
            date,
            project: Arc::clone(project),
            usage,
            cost,
            model,
            missing_pricing_model,
            message_id: raw.message_id,
            request_id: raw.request_id,
            is_sidechain: raw.is_sidechain,
        });
        return;
    }
    out.push(DailyLoadedEntry {
        timestamp_ms: raw.timestamp_ms,
        date: Arc::clone(&date),
        project: Arc::clone(project),
        usage,
        cost,
        model,
        missing_pricing_model,
        message_id: raw.message_id.clone(),
        request_id: raw.request_id.clone(),
        is_sidechain: raw.is_sidechain,
    });
    for (index, (advisor_model, advisor_usage)) in raw.advisors.iter().enumerate() {
        let (cost, missing_pricing_model) = cost_and_missing_model_for_usage(
            Some(advisor_model),
            *advisor_usage,
            None,
            mode,
            pricing,
        );
        out.push(DailyLoadedEntry {
            timestamp_ms: raw.timestamp_ms,
            date: Arc::clone(&date),
            project: Arc::clone(project),
            usage: *advisor_usage,
            cost,
            model: Some(advisor_model.clone()),
            missing_pricing_model,
            message_id: raw
                .message_id
                .as_ref()
                .map(|message_id| format!("{message_id}:advisor:{index}")),
            request_id: raw.request_id.clone(),
            is_sidechain: raw.is_sidechain,
        });
    }
}

fn write_daily_raw_entry(writer: &mut cache::Writer, entry: &DailyRawEntry) {
    writer.push_i64(entry.timestamp_ms);
    cache::write_token_usage(writer, entry.usage);
    writer.push_opt_f64(entry.cost_usd);
    writer.push_opt_str(entry.model.as_deref());
    writer.push_opt_str(entry.message_id.as_deref());
    writer.push_opt_str(entry.request_id.as_deref());
    writer.push_opt_bool(entry.is_sidechain);
    writer.push_u32(entry.advisors.len() as u32);
    for (model, usage) in &entry.advisors {
        writer.push_str(model);
        cache::write_token_usage(writer, *usage);
    }
}

fn read_daily_raw_entry(reader: &mut cache::Reader<'_>) -> Option<DailyRawEntry> {
    let timestamp_ms = reader.read_i64()?;
    let usage = cache::read_token_usage(reader)?;
    let cost_usd = reader.read_opt_f64()?;
    let model = reader.read_opt_str()?;
    let message_id = reader.read_opt_str()?;
    let request_id = reader.read_opt_str()?;
    let is_sidechain = reader.read_opt_bool()?;
    let advisor_count = reader.read_u32()? as usize;
    let mut advisors = Vec::with_capacity(advisor_count.min(1024));
    for _ in 0..advisor_count {
        let model = reader.read_str()?;
        let usage = cache::read_token_usage(reader)?;
        advisors.push((model, usage));
    }
    Some(DailyRawEntry {
        timestamp_ms,
        usage,
        cost_usd,
        model,
        message_id,
        request_id,
        is_sidechain,
        advisors,
    })
}

fn is_valid_daily_usage_entry(data: &DailyUsageEntry) -> bool {
    if data
        .version
        .as_deref()
        .is_some_and(|version| !is_semver_prefix(version))
    {
        return false;
    }
    if data
        .session_id
        .as_deref()
        .is_some_and(|session_id| session_id.is_empty())
    {
        return false;
    }
    if data
        .request_id
        .as_deref()
        .is_some_and(|request_id| request_id.is_empty())
    {
        return false;
    }
    if data
        .message
        .id
        .as_deref()
        .is_some_and(|message_id| message_id.is_empty())
    {
        return false;
    }
    if data
        .message
        .model
        .as_deref()
        .is_some_and(|model| model.is_empty())
    {
        return false;
    }
    true
}
fn daily_usage_token_total(entry: &DailyLoadedEntry) -> u64 {
    entry.usage.input_tokens
        + entry.usage.output_tokens
        + entry.usage.cache_creation_token_count()
        + entry.usage.cache_read_input_tokens
}

/// Result of pushing one entry through the message/request dedup map, so
/// streaming callers can adjust running aggregations.
pub(super) enum DailyDedupOutcome {
    Added(usize),
    Replaced {
        index: usize,
        previous: Box<DailyLoadedEntry>,
    },
    Duplicate,
}

pub(super) fn push_deduped_daily_entry(
    entry: DailyLoadedEntry,
    deduped_indexes: &mut FxHashMap<u64, SmallIndexVec>,
    deduped: &mut Vec<DailyLoadedEntry>,
) -> DailyDedupOutcome {
    let dedupe_lookup = entry.message_id.as_deref().map(|message_id| {
        let request_id = entry.request_id.as_deref();
        let exact_hash = usage_dedupe_hash(message_id, request_id);
        let existing_index = deduped_indexes
            .get(&exact_hash)
            .and_then(|indexes| {
                indexes.iter().copied().find(|&index| {
                    deduped[index].message_id.as_deref() == Some(message_id)
                        && deduped[index].request_id.as_deref() == request_id
                })
            })
            .or_else(|| {
                // /btw sidechain logs can replay parent messages with new request IDs.
                let message_hash = usage_dedupe_hash(message_id, None);
                let candidate_is_sidechain = is_sidechain_daily_entry(&entry);
                deduped_indexes.get(&message_hash).and_then(|indexes| {
                    indexes.iter().copied().find(|&index| {
                        deduped[index].message_id.as_deref() == Some(message_id)
                            && (candidate_is_sidechain || is_sidechain_daily_entry(&deduped[index]))
                    })
                })
            });
        (exact_hash, existing_index)
    });

    if let Some((hash, Some(index))) = dedupe_lookup {
        if should_replace_deduped_daily_entry(&entry, &deduped[index]) {
            let previous = Box::new(std::mem::replace(&mut deduped[index], entry));
            push_deduped_daily_index(deduped_indexes, hash, index);
            if let Some(message_id) = deduped[index].message_id.as_deref() {
                push_deduped_daily_index(
                    deduped_indexes,
                    usage_dedupe_hash(message_id, None),
                    index,
                );
            }
            return DailyDedupOutcome::Replaced { index, previous };
        }
        return DailyDedupOutcome::Duplicate;
    }

    let index = deduped.len();
    deduped.push(entry);
    if let Some((hash, None)) = dedupe_lookup {
        push_deduped_daily_index(deduped_indexes, hash, index);
        if let Some(message_id) = deduped[index].message_id.as_deref() {
            push_deduped_daily_index(deduped_indexes, usage_dedupe_hash(message_id, None), index);
        }
    }
    DailyDedupOutcome::Added(index)
}

fn should_replace_deduped_daily_entry(
    candidate: &DailyLoadedEntry,
    existing: &DailyLoadedEntry,
) -> bool {
    let candidate_is_sidechain = is_sidechain_daily_entry(candidate);
    let existing_is_sidechain = is_sidechain_daily_entry(existing);
    if candidate_is_sidechain != existing_is_sidechain {
        return existing_is_sidechain;
    }

    let candidate_total = daily_usage_token_total(candidate);
    let existing_total = daily_usage_token_total(existing);
    if candidate_total != existing_total {
        return candidate_total > existing_total;
    }
    if candidate.cost != existing.cost {
        return candidate.cost > existing.cost;
    }
    candidate.usage.speed.is_some() && existing.usage.speed.is_none()
}

fn is_sidechain_daily_entry(entry: &DailyLoadedEntry) -> bool {
    entry.is_sidechain == Some(true)
}

fn push_deduped_daily_index(
    deduped_indexes: &mut FxHashMap<u64, SmallIndexVec>,
    hash: u64,
    index: usize,
) {
    let indexes = deduped_indexes.entry(hash).or_default();
    if !indexes.contains(&index) {
        indexes.push(index);
    }
}

#[derive(Default)]
pub(crate) struct DailyAccumulator {
    counts: TokenCounts,
    cost: f64,
    models: Vec<String>,
    breakdowns: Vec<ModelBreakdown>,
    breakdown_indexes: FxHashMap<String, usize>,
}

impl DailyAccumulator {
    pub(crate) fn add_entry(&mut self, entry: &DailyLoadedEntry) {
        self.counts.add_usage(entry.usage);
        self.cost += entry.cost;
        if let Some(model) = &entry.model {
            let model = crate::model_aliases::resolve_model_name(model);
            let index = if let Some(index) = self.breakdown_indexes.get(model.as_ref()) {
                *index
            } else {
                let model = model.into_owned();
                let index = self.breakdowns.len();
                self.breakdown_indexes.insert(model.clone(), index);
                self.models.push(model.clone());
                self.breakdowns.push(ModelBreakdown {
                    model_name: model,
                    ..ModelBreakdown::default()
                });
                index
            };
            let breakdown = &mut self.breakdowns[index];
            breakdown.input_tokens += entry.usage.input_tokens;
            breakdown.output_tokens += entry.usage.output_tokens;
            breakdown.cache_creation_tokens += entry.usage.cache_creation_token_count();
            breakdown.cache_read_tokens += entry.usage.cache_read_input_tokens;
            breakdown.cost += entry.cost;
            if entry.missing_pricing_model.is_some() {
                breakdown.missing_pricing = true;
            }
        }
    }

    fn into_summary(self) -> UsageSummary {
        self.to_summary()
    }

    pub(crate) fn to_summary(&self) -> UsageSummary {
        let mut breakdowns = self.breakdowns.clone();
        breakdowns.sort_by(|a, b| b.cost.total_cmp(&a.cost));
        UsageSummary {
            date: None,
            month: None,
            week: None,
            session_id: None,
            project_path: None,
            last_activity: None,
            first_activity: None,
            input_tokens: self.counts.input_tokens,
            output_tokens: self.counts.output_tokens,
            cache_creation_tokens: self.counts.cache_creation_tokens,
            cache_read_tokens: self.counts.cache_read_tokens,
            extra_total_tokens: 0,
            total_cost: self.cost,
            credits: None,
            message_count: None,
            models_used: self.models.clone(),
            model_breakdowns: breakdowns,
            project: None,
            versions: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DailyLoadedEntry, DailyRawEntry, push_deduped_daily_entry, read_daily_raw_entry,
        write_daily_raw_entry,
    };
    use crate::TokenUsageRaw;
    use crate::cache::{Reader, Writer};

    #[test]
    fn report_cache_key_tracks_replacement_with_preserved_size_and_mtime() {
        use std::{
            fs,
            time::{Duration, UNIX_EPOCH},
        };
        let fixture = turbotokens_test_support::fs_fixture!({ "log.jsonl": "old\n" });
        let path = fixture.path("log.jsonl");
        let fixed_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let set_time = |path: &std::path::Path| {
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(fixed_time))
                .unwrap();
        };
        set_time(&path);
        let shared = crate::cli::SharedArgs {
            offline: true,
            ..Default::default()
        };
        let key = || {
            super::daily_report_key(
                &shared,
                std::slice::from_ref(&path),
                None,
                false,
                &jiff::tz::TimeZone::UTC,
            )
        };
        let before_key = key();
        let before = fs::metadata(&path).unwrap();
        let replacement = fixture.path("replacement.tmp");
        fs::write(&replacement, "new\n").unwrap();
        set_time(&replacement);
        fs::rename(&replacement, &path).unwrap();
        let after = fs::metadata(&path).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert!(before_key.is_some());
        assert_ne!(before_key, key());
        fs::remove_file(&path).unwrap();
        assert!(key().is_none());
    }

    #[test]
    fn report_cache_key_tracks_resolved_timezone() {
        let shared = crate::cli::SharedArgs {
            offline: true,
            ..Default::default()
        };
        let utc = jiff::tz::TimeZone::UTC;
        let los_angeles = jiff::tz::TimeZone::get("America/Los_Angeles").unwrap();
        let utc_key = super::daily_report_key(&shared, &[], None, false, &utc);
        let local_key = super::daily_report_key(&shared, &[], None, false, &los_angeles);

        assert!(utc_key.is_some());
        assert!(local_key.is_some());
        assert_ne!(utc_key, local_key);
        assert_eq!(
            utc_key,
            super::daily_report_key(&shared, &[], None, false, &utc)
        );
    }

    #[test]
    fn report_cache_key_tracks_posix_timezone_rules() {
        let shared = crate::cli::SharedArgs {
            offline: true,
            ..Default::default()
        };
        let fixed = jiff::tz::TimeZone::posix("EST5").unwrap();
        let seasonal = jiff::tz::TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();

        assert_ne!(
            super::daily_report_key(&shared, &[], None, false, &fixed),
            super::daily_report_key(&shared, &[], None, false, &seasonal),
        );
    }

    #[test]
    fn roundtrips_daily_raw_entry_cache_encoding() {
        let entries = [
            DailyRawEntry {
                timestamp_ms: 1_775_000_000_000,
                usage: TokenUsageRaw {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                    speed: None,
                    cache_creation: None,
                },
                cost_usd: None,
                model: None,
                message_id: None,
                request_id: None,
                is_sidechain: None,
                advisors: Vec::new(),
            },
            DailyRawEntry {
                timestamp_ms: i64::MIN + 1,
                usage: TokenUsageRaw {
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_creation_input_tokens: 3,
                    cache_read_input_tokens: 4,
                    speed: Some(crate::Speed::Fast),
                    cache_creation: Some(crate::CacheCreationRaw {
                        ephemeral_5m_input_tokens: 5,
                        ephemeral_1h_input_tokens: 6,
                    }),
                },
                cost_usd: Some(1.25),
                model: Some("claude-sonnet-4-日本語".to_string()),
                message_id: Some("msg-β".to_string()),
                request_id: Some("req-✓".to_string()),
                is_sidechain: Some(true),
                advisors: vec![
                    (
                        "advisor-model".to_string(),
                        TokenUsageRaw {
                            input_tokens: 10,
                            output_tokens: 20,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                            speed: None,
                            cache_creation: None,
                        },
                    ),
                    (
                        "advisor-日本語".to_string(),
                        TokenUsageRaw {
                            input_tokens: 0,
                            output_tokens: 1,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                            speed: Some(crate::Speed::Standard),
                            cache_creation: None,
                        },
                    ),
                ],
            },
        ];

        let mut writer = Writer::new();
        for entry in &entries {
            write_daily_raw_entry(&mut writer, entry);
        }
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        for entry in &entries {
            let decoded = read_daily_raw_entry(&mut reader).expect("entry decodes");
            assert_eq!(&decoded, entry);
        }
        assert!(reader.finish().is_some());
    }

    #[test]
    fn keeps_parent_daily_usage_when_sidechain_replays_message_with_new_request_id() {
        let mut deduped_indexes = Default::default();
        let mut deduped = Vec::new();

        push_deduped_daily_entry(
            daily_loaded_entry(DailyEntryFixture {
                message_id: "msg-parent",
                request_id: "req-parent",
                is_sidechain: false,
                cache_read_tokens: 20,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_daily_entry(
            daily_loaded_entry(DailyEntryFixture {
                message_id: "msg-parent",
                request_id: "req-sidechain-replay",
                is_sidechain: true,
                cache_read_tokens: 50_000,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_daily_entry(
            daily_loaded_entry(DailyEntryFixture {
                message_id: "msg-sidechain-answer",
                request_id: "req-sidechain-answer",
                is_sidechain: true,
                cache_read_tokens: 700,
                output_tokens: 30,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].message_id.as_deref(), Some("msg-parent"));
        assert_eq!(deduped[0].request_id.as_deref(), Some("req-parent"));
        assert_eq!(deduped[0].usage.cache_read_input_tokens, 20);
        assert_eq!(
            deduped[1].message_id.as_deref(),
            Some("msg-sidechain-answer")
        );
        assert_eq!(deduped[1].usage.cache_read_input_tokens, 700);
    }

    #[test]
    fn refreshes_daily_dedupe_indexes_when_parent_replaces_sidechain_replay() {
        let mut deduped_indexes = Default::default();
        let mut deduped = Vec::new();

        for fixture in [
            DailyEntryFixture {
                message_id: "msg-parent",
                request_id: "req-sidechain-replay",
                is_sidechain: true,
                cache_read_tokens: 50_000,
                output_tokens: 10,
            },
            DailyEntryFixture {
                message_id: "msg-parent",
                request_id: "req-parent",
                is_sidechain: false,
                cache_read_tokens: 20,
                output_tokens: 10,
            },
            DailyEntryFixture {
                message_id: "msg-parent",
                request_id: "req-parent",
                is_sidechain: false,
                cache_read_tokens: 5,
                output_tokens: 5,
            },
        ] {
            push_deduped_daily_entry(
                daily_loaded_entry(fixture),
                &mut deduped_indexes,
                &mut deduped,
            );
        }

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].request_id.as_deref(), Some("req-parent"));
        assert_eq!(deduped[0].usage.cache_read_input_tokens, 20);
    }

    #[test]
    fn roundtrips_cached_summaries() {
        let summaries = vec![
            crate::UsageSummary {
                date: Some("2026-07-28".to_string()),
                month: None,
                week: None,
                session_id: None,
                project_path: None,
                last_activity: None,
                first_activity: None,
                input_tokens: 123,
                output_tokens: 45,
                cache_creation_tokens: 6,
                cache_read_tokens: 7_890,
                extra_total_tokens: 0,
                total_cost: 1.25,
                credits: None,
                message_count: Some(3),
                models_used: vec!["claude-sonnet-4-20250514".to_string()],
                model_breakdowns: vec![crate::ModelBreakdown {
                    model_name: "claude-sonnet-4-20250514".to_string(),
                    input_tokens: 123,
                    output_tokens: 45,
                    cache_creation_tokens: 6,
                    cache_read_tokens: 7_890,
                    extra_total_tokens: 0,
                    cost: 1.25,
                    missing_pricing: true,
                }],
                project: Some("proj-日本語".to_string()),
                versions: Some(vec!["1.2.3".to_string()]),
            },
            crate::UsageSummary {
                date: None,
                month: None,
                week: None,
                session_id: None,
                project_path: None,
                last_activity: None,
                first_activity: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                extra_total_tokens: 0,
                total_cost: 0.0,
                credits: Some(2.5),
                message_count: None,
                models_used: Vec::new(),
                model_breakdowns: Vec::new(),
                project: None,
                versions: None,
            },
        ];

        let decoded = super::decode_summaries(&super::encode_summaries(&summaries))
            .expect("summaries decode");

        assert_eq!(decoded.len(), 2);
        let first = &decoded[0];
        assert_eq!(first.date.as_deref(), Some("2026-07-28"));
        assert_eq!(first.input_tokens, 123);
        assert_eq!(first.total_cost, 1.25);
        assert_eq!(first.message_count, Some(3));
        assert_eq!(first.models_used, ["claude-sonnet-4-20250514"]);
        assert_eq!(first.model_breakdowns.len(), 1);
        assert!(first.model_breakdowns[0].missing_pricing);
        assert_eq!(first.project.as_deref(), Some("proj-日本語"));
        assert_eq!(first.versions.as_deref(), Some(&["1.2.3".to_string()][..]));
        let second = &decoded[1];
        assert_eq!(second.date, None);
        assert_eq!(second.credits, Some(2.5));
        assert!(second.models_used.is_empty());
        assert!(super::decode_summaries(&[0xff, 0x00]).is_none());
    }

    #[test]
    fn propagates_sidechain_metadata_from_agent_progress_lines() {
        let data = super::parse_daily_usage_line(
            br#"{"data":{"message":{"timestamp":"2026-03-29T07:00:00.000Z","requestId":"req-sidechain","isSidechain":true,"message":{"usage":{"input_tokens":0,"output_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":20},"model":"claude-sonnet-4-20250514","id":"msg-sidechain"}}}}"#,
        )
        .unwrap();

        assert_eq!(data.is_sidechain, Some(true));
    }

    #[test]
    fn scans_mixed_whitespace_records_in_file_order() {
        let record = |id, output| {
            format!(
                r#"{{"timestamp":"2026-09-08T01:00:00.000Z","message":{{"id":"msg-{id}","model":"claude-sonnet-4","usage":{{"input_tokens":123,"output_tokens":{output}}}}}}}"#
            )
        };
        let bytes = format!(
            "{}\r\n{}\n{}",
            record(1, 10).replace("\":", "\" \t: \t"),
            record(2, 20),
            record(3, 30).replace("\":", "\": "),
        );

        let scanned = super::scan_daily_bytes(bytes.as_bytes());

        assert_eq!(scanned.entries.len(), 2);
        assert_eq!(scanned.tail_entries.len(), 1);
        for (index, entry) in scanned
            .entries
            .iter()
            .chain(&scanned.tail_entries)
            .enumerate()
        {
            assert_eq!(entry.message_id, Some(format!("msg-{}", index + 1)));
            assert_eq!(entry.usage.input_tokens, 123);
            assert_eq!(entry.usage.output_tokens, ((index + 1) * 10) as u64);
        }
        assert_eq!(scanned.consumed as usize, bytes.rfind('\n').unwrap() + 1);
    }

    #[test]
    fn accepts_whitespace_in_agent_progress_usage() {
        let compact = r#"{"data":{"message":{"timestamp":"2026-09-08T01:00:00.000Z","requestId":"req-1","message":{"id":"msg-1","model":"claude-sonnet-4","usage":{"input_tokens":123,"output_tokens":45}}}}}"#;
        let expected = super::scan_daily_bytes(compact.as_bytes()).tail_entries;
        assert_eq!(expected.len(), 1);

        let spaced = compact.replace("\":", "\" \t: \t");

        assert_eq!(
            super::scan_daily_bytes(spaced.as_bytes()).tail_entries,
            expected
        );
    }

    struct DailyEntryFixture {
        message_id: &'static str,
        request_id: &'static str,
        is_sidechain: bool,
        cache_read_tokens: u64,
        output_tokens: u64,
    }

    fn daily_loaded_entry(fixture: DailyEntryFixture) -> DailyLoadedEntry {
        DailyLoadedEntry {
            timestamp_ms: 0,
            date: Arc::from("2026-03-29"),
            project: Arc::from("project-a"),
            usage: TokenUsageRaw {
                input_tokens: 0,
                output_tokens: fixture.output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: fixture.cache_read_tokens,
                speed: None,
                cache_creation: None,
            },
            cost: 0.0,
            model: Some("claude-sonnet-4-20250514".to_string()),
            missing_pricing_model: None,
            message_id: Some(fixture.message_id.to_string()),
            request_id: Some(fixture.request_id.to_string()),
            is_sidechain: Some(fixture.is_sidechain),
        }
    }
}
