use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

use crate::{Result, collect_files_with_extension};

pub(super) const GROK_HOME_ENV: &str = "GROK_HOME";
const UNIFIED_LOG_FILE: &str = "unified.jsonl";
const EVENTS_FILE: &str = "events.jsonl";

fn homes() -> Vec<PathBuf> {
    if let Ok(value) = env::var(GROK_HOME_ENV) {
        return value
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect();
    }
    crate::home::home_dir()
        .map(|home| home.join(".grok"))
        .filter(|path| path.is_dir())
        .into_iter()
        .collect()
}

pub(super) fn data_dirs() -> Vec<PathBuf> {
    homes()
}

pub(super) fn discover_usage_logs() -> Vec<PathBuf> {
    let mut files = homes()
        .into_iter()
        .map(|home| home.join("logs").join(UNIFIED_LOG_FILE))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files
}

pub(super) fn discover_event_files() -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for home in homes() {
        let mut candidates = Vec::new();
        collect_files_with_extension(&home.join("sessions"), "jsonl", &mut candidates);
        for file in candidates {
            if is_event_file(&file) && seen.insert(file.clone()) {
                files.push(file);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn is_event_file(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some(EVENTS_FILE)
}

#[cfg(test)]
pub(super) static GROK_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use turbotokens_test_support::{EnvVarGuard, fs_fixture};

    #[test]
    fn discovers_only_grok_usage_and_turn_event_logs() {
        let _guard = GROK_HOME_LOCK.lock().unwrap();
        let fixture = fs_fixture!({
            "logs/unified.jsonl": "{}\n",
            "logs/other.jsonl": "{}\n",
            "sessions/workspace/session-a/events.jsonl": "{}\n",
            "sessions/workspace/session-a/chat_history.jsonl": "{}\n",
        });
        let _env = EnvVarGuard::set(GROK_HOME_ENV, fixture.root());

        assert_eq!(
            discover_usage_logs(),
            vec![fixture.path("logs/unified.jsonl")]
        );
        assert_eq!(
            discover_event_files().unwrap(),
            vec![fixture.path("sessions/workspace/session-a/events.jsonl")]
        );
    }
}
