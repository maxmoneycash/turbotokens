use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use crate::{
    Result, UsageSummary,
    cli::SharedArgs,
    daily::{DailyAccumulator, DailyLoadedEntry},
    fast::FxHashMap,
    paths::claude_paths,
    watch::{WatchIndex, WatchOutcome},
};

/// Resident daily index for the daemon: the shared incremental scanner plus
/// per-date and per-(date, project) accumulators, maintained entry by entry
/// so daily report rows are served without rescanning the logs.
///
/// Accumulators are updated incrementally for accepted entries and rebuilt
/// for the affected date when a dedup replay replaces an entry, keeping the
/// rows identical to a full `load_daily_summaries` aggregation.
pub struct ResidentIndex {
    paths: Vec<PathBuf>,
    single_thread: bool,
    watch: WatchIndex,
    by_date: BTreeMap<String, DailyAccumulator>,
    by_date_project: BTreeMap<(String, Arc<str>), DailyAccumulator>,
    /// Date → deduped entry indices, used to rebuild accumulators after a
    /// dedup replacement. Entries are re-checked against their date at
    /// rebuild time because a replacement can move an entry across dates.
    date_entries: FxHashMap<String, Vec<usize>>,
}

impl ResidentIndex {
    pub fn new(shared: &SharedArgs) -> Result<Self> {
        Ok(Self::with_paths(shared, claude_paths()?))
    }

    /// Builds an index over explicit data directories (test hook; production
    /// goes through [`ResidentIndex::new`]).
    #[doc(hidden)]
    pub fn with_paths(shared: &SharedArgs, paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            single_thread: shared.single_thread,
            watch: WatchIndex::new(shared),
            by_date: BTreeMap::new(),
            by_date_project: BTreeMap::new(),
            date_entries: FxHashMap::default(),
        }
    }

    /// Reads every known log file, in parallel by default.
    pub fn seed(&mut self) {
        let mut outcomes = Vec::new();
        self.watch
            .seed(&self.paths, self.single_thread, &mut |outcome| {
                outcomes.push(outcome);
            });
        for outcome in outcomes {
            self.apply(outcome);
        }
    }

    /// Picks up new files, appends, and rotations in the data directories.
    pub fn poll(&mut self) {
        let mut outcomes = Vec::new();
        self.watch
            .poll_paths(&self.paths, &mut |outcome| outcomes.push(outcome));
        for outcome in outcomes {
            self.apply(outcome);
        }
    }

    /// Daily report rows in the same order `load_daily_summaries` returns
    /// them: ascending by date, then by project when grouped.
    pub fn daily_rows(&self, project: Option<&str>, group_by_project: bool) -> Vec<UsageSummary> {
        if group_by_project {
            return self
                .by_date_project
                .iter()
                .filter(|((_, row_project), _)| {
                    project.is_none_or(|filter| row_project.as_ref() == filter)
                })
                .map(|((date, row_project), group)| {
                    let mut summary = group.to_summary();
                    summary.date = Some(date.clone());
                    summary.project = Some(row_project.to_string());
                    summary
                })
                .collect();
        }
        self.by_date
            .iter()
            .map(|(date, group)| {
                let mut summary = group.to_summary();
                summary.date = Some(date.clone());
                summary
            })
            .collect()
    }

    pub fn files_watched(&self) -> usize {
        self.watch.cursors.len()
    }

    /// Source directories represented by this index, for daemon compatibility.
    pub fn source_paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn entries_indexed(&self) -> usize {
        self.watch.deduped.len()
    }

    fn apply(&mut self, outcome: WatchOutcome) {
        match outcome {
            WatchOutcome::Reset => {
                self.by_date.clear();
                self.by_date_project.clear();
                self.date_entries.clear();
                for index in 0..self.watch.deduped.len() {
                    self.add_entry(index);
                }
            }
            WatchOutcome::Added { index, .. } => {
                self.add_entry(index);
            }
            WatchOutcome::Replaced {
                index, previous, ..
            } => {
                let (date, project) = {
                    let entry = &self.watch.deduped[index];
                    (entry.date.to_string(), Arc::clone(&entry.project))
                };
                if date.as_str() != previous.date.as_ref() {
                    self.date_entries
                        .entry(date.clone())
                        .or_default()
                        .push(index);
                }
                self.rebuild_date(&date, std::slice::from_ref(&project));
                if previous.date.as_ref() != date.as_str() || previous.project != project {
                    self.rebuild_date(
                        previous.date.as_ref(),
                        std::slice::from_ref(&Arc::clone(&previous.project)),
                    );
                }
            }
        }
    }

    fn add_entry(&mut self, index: usize) {
        let entry = &self.watch.deduped[index];
        let date = entry.date.to_string();
        let project = Arc::clone(&entry.project);
        self.by_date
            .entry(date.clone())
            .or_default()
            .add_entry(entry);
        self.by_date_project
            .entry((date.clone(), project))
            .or_default()
            .add_entry(entry);
        self.date_entries.entry(date).or_default().push(index);
    }

    /// Rebuilds one date's accumulators from the resident entries so a dedup
    /// replacement leaves rows identical to a fresh aggregation.
    fn rebuild_date(&mut self, date: &str, projects: &[Arc<str>]) {
        let Some(indices) = self.date_entries.get(date) else {
            return;
        };
        let mut date_group = DailyAccumulator::default();
        let mut project_groups = projects
            .iter()
            .map(|project| (Arc::clone(project), DailyAccumulator::default(), 0usize))
            .collect::<Vec<_>>();
        let mut date_count = 0;
        for &index in indices {
            let entry: &DailyLoadedEntry = &self.watch.deduped[index];
            if entry.date.as_ref() != date {
                continue;
            }
            date_group.add_entry(entry);
            date_count += 1;
            for (project, group, count) in &mut project_groups {
                if *project == entry.project {
                    group.add_entry(entry);
                    *count += 1;
                }
            }
        }
        if date_count == 0 {
            self.by_date.remove(date);
        } else {
            self.by_date.insert(date.to_string(), date_group);
        }
        for (project, group, count) in project_groups {
            let key = (date.to_string(), Arc::clone(&project));
            if count == 0 {
                self.by_date_project.remove(&key);
            } else {
                self.by_date_project.insert(key, group);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use turbotokens_test_support::fs_fixture;

    use super::*;
    use crate::cli::CostMode;

    fn usage_line(message_id: &str, output_tokens: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-07-27T18:00:00.000Z","version":"1.2.3","sessionId":"sess-1","message":{{"id":"{message_id}","model":"claude-sonnet-4","usage":{{"input_tokens":100,"output_tokens":{output_tokens},"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}},"requestId":"req-{message_id}","costUSD":0.01}}"#
        )
    }

    fn shared() -> SharedArgs {
        SharedArgs {
            mode: CostMode::Display,
            timezone: Some("UTC".to_string()),
            ..SharedArgs::default()
        }
    }

    #[test]
    fn replaces_changed_history_instead_of_retaining_old_contributions() {
        let first = usage_line("msg-1", 20);
        let second = usage_line("msg-2", 30);
        let initial = format!("{first}\n{second}\n");
        let replacements = [
            (format!("{}\n{second}\n", usage_line("msg-3", 40)), 300),
            (
                format!("{}\n{second}\n", usage_line("msg-3", 400))
                    .replace("2026-07-27", "2026-07-28"),
                660,
            ),
            (format!("{first}\n"), 135),
            (String::new(), 0),
        ];
        for single_thread in [true, false] {
            for (replacement, expected_tokens) in &replacements {
                let fixture = fs_fixture!({
                    "projects/proj-a/sess-1.jsonl": initial.clone(),
                });
                let paths = vec![fixture.root().to_path_buf()];
                let mut shared = shared();
                shared.single_thread = single_thread;
                let mut index = ResidentIndex::with_paths(&shared, paths.clone());
                index.seed();
                let path = fixture.path("projects/proj-a/sess-1.jsonl");
                std::fs::write(&path, replacement).unwrap();
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
                    .unwrap();
                index.poll();
                let rows = index.daily_rows(None, false);
                assert_eq!(
                    rows.iter().map(|row| row.total_tokens()).sum::<u64>(),
                    *expected_tokens
                );
                let mut fresh = ResidentIndex::with_paths(&shared, paths);
                fresh.seed();
                for grouped in [false, true] {
                    assert_eq!(
                        serde_json::to_value(index.daily_rows(None, grouped)).unwrap(),
                        serde_json::to_value(fresh.daily_rows(None, grouped)).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn deletion_and_restoration_reselect_duplicate_winners_across_files() {
        let lower = format!("{}\n", usage_line("shared", 20));
        let higher = format!("{}\n", usage_line("shared", 250));
        let fixture = fs_fixture!({
            "projects/proj-a/low.jsonl": lower.clone(),
            "projects/proj-b/high.jsonl": higher.clone(),
        });
        let mut index = ResidentIndex::with_paths(&shared(), vec![fixture.root().to_path_buf()]);
        index.seed();
        assert_eq!(index.daily_rows(None, false)[0].total_tokens(), 365);
        assert_eq!(
            index.daily_rows(None, true)[0].project.as_deref(),
            Some("proj-b")
        );
        let high_path = fixture.path("projects/proj-b/high.jsonl");
        std::fs::remove_file(&high_path).unwrap();
        index.poll();
        assert_eq!(index.daily_rows(None, false)[0].total_tokens(), 135);
        assert_eq!(
            index.daily_rows(None, true)[0].project.as_deref(),
            Some("proj-a")
        );
        assert_eq!(index.files_watched(), 1);
        std::fs::write(&high_path, &higher).unwrap();
        index.poll();
        assert_eq!(index.daily_rows(None, false)[0].total_tokens(), 365);
        assert_eq!(
            index.daily_rows(None, true)[0].project.as_deref(),
            Some("proj-b")
        );
        assert_eq!(index.entries_indexed(), 1);
        std::fs::remove_file(&high_path).unwrap();
        std::fs::remove_file(fixture.path("projects/proj-a/low.jsonl")).unwrap();
        index.poll();
        assert!(index.daily_rows(None, false).is_empty());
        assert!(index.daily_rows(None, true).is_empty());
        assert_eq!(index.files_watched(), 0);
        assert_eq!(index.entries_indexed(), 0);
    }

    #[test]
    fn seeds_and_serves_daily_rows() {
        let fixture = fs_fixture!({
            "projects/proj-a/sess-1.jsonl": format!("{}\n{}\n", usage_line("msg-1", 20), usage_line("msg-2", 30)),
            "projects/proj-b/sess-2.jsonl": format!("{}\n", usage_line("msg-3", 40)),
        });
        let mut index = ResidentIndex::with_paths(&shared(), vec![fixture.root().to_path_buf()]);

        index.seed();

        let rows = index.daily_rows(None, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date.as_deref(), Some("2026-07-27"));
        assert_eq!(rows[0].input_tokens, 300);
        assert_eq!(rows[0].output_tokens, 90);
        assert!((rows[0].total_cost - 0.03).abs() < 1e-9);

        let grouped = index.daily_rows(None, true);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].project.as_deref(), Some("proj-a"));
        assert_eq!(grouped[1].project.as_deref(), Some("proj-b"));

        let filtered = index.daily_rows(Some("proj-b"), true);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].output_tokens, 40);
    }

    #[test]
    fn picks_up_appended_lines_on_poll() {
        let fixture = fs_fixture!({
            "projects/proj-a/sess-1.jsonl": format!("{}\n", usage_line("msg-1", 20)),
        });
        let path = fixture.path("projects/proj-a/sess-1.jsonl");
        let mut index = ResidentIndex::with_paths(&shared(), vec![fixture.root().to_path_buf()]);
        index.seed();
        assert_eq!(index.daily_rows(None, false)[0].output_tokens, 20);

        std::fs::write(
            &path,
            format!("{}\n{}\n", usage_line("msg-1", 20), usage_line("msg-2", 30)),
        )
        .unwrap();
        index.poll();

        let rows = index.daily_rows(None, false);
        assert_eq!(rows[0].input_tokens, 200);
        assert_eq!(rows[0].output_tokens, 50);
    }

    #[test]
    fn adjusts_accumulators_when_a_replay_replaces_an_entry() {
        let fixture = fs_fixture!({
            "projects/proj-a/sess-1.jsonl": format!("{}\n", usage_line("msg-1", 20)),
        });
        let path = fixture.path("projects/proj-a/sess-1.jsonl");
        let mut index = ResidentIndex::with_paths(&shared(), vec![fixture.root().to_path_buf()]);
        index.seed();

        // A more complete replay of the same message replaces the entry.
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                usage_line("msg-1", 20),
                usage_line("msg-1", 250)
            ),
        )
        .unwrap();
        index.poll();

        let rows = index.daily_rows(None, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_tokens, 100);
        assert_eq!(rows[0].output_tokens, 250);
        assert!((rows[0].total_cost - 0.01).abs() < 1e-9);

        let grouped = index.daily_rows(None, true);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].output_tokens, 250);
    }
}
