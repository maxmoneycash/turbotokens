//! `turbotokens doctor` — diagnostic checklist for data directories, cache health,
//! and pricing setup. Exits 0 for warnings; only hard failures are non-zero.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use crate::{
    Result, adapter::claude, cli::SharedArgs, cli_error, home, pricing::PricingMap,
    print_json_or_jq, wants_json,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Status {
    Ok,
    Warn,
    Fail,
    Info,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Info => "info",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
            Self::Info => "•",
        }
    }
}

struct Check {
    name: &'static str,
    status: Status,
    detail: String,
    hint: Option<String>,
}

impl Check {
    fn new(name: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            name,
            status,
            detail: detail.into(),
            hint: None,
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

pub(super) fn run(shared: &SharedArgs) -> Result<()> {
    let checks = collect_checks(shared);
    let hard_failure = checks.iter().any(|check| check.status == Status::Fail);

    if wants_json(shared) {
        print_json_or_jq(checks_json(&checks), shared.jq.as_deref(), false)?;
    } else {
        println!("turbotokens doctor\n");
        for check in &checks {
            println!("{} {}: {}", check.status.symbol(), check.name, check.detail);
            if let Some(hint) = &check.hint {
                println!("  fix: {hint}");
            }
        }
    }

    if hard_failure {
        return Err(cli_error("doctor found hard failures (see above)"));
    }
    Ok(())
}

fn collect_checks(shared: &SharedArgs) -> Vec<Check> {
    vec![
        Check::new("version", Status::Ok, env!("TURBOTOKENS_VERSION")),
        check_claude_data(),
        check_codex_data(),
        check_other_agents(),
        check_cache(),
        check_daemon(),
        check_config(shared),
        check_pricing(),
    ]
}

fn check_claude_data() -> Check {
    match claude::claude_paths() {
        Ok(paths) if !paths.is_empty() => {
            let files = claude::usage_files(&paths, None);
            let bytes = files
                .iter()
                .filter_map(|file| fs::metadata(file).ok())
                .map(|metadata| metadata.len())
                .sum::<u64>();
            Check::new(
                "claude data",
                Status::Ok,
                format!(
                    "{} ({} JSONL files, {})",
                    paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    files.len(),
                    format_bytes(bytes)
                ),
            )
        }
        Ok(_) => Check::new(
            "claude data",
            Status::Warn,
            "no Claude data directories found",
        )
        .with_hint("Run Claude Code once, or point CLAUDE_CONFIG_DIR at its config directory."),
        Err(error) => Check::new("claude data", Status::Warn, error.to_string())
            .with_hint("Fix CLAUDE_CONFIG_DIR: each entry must contain a 'projects/' directory."),
    }
}

fn check_codex_data() -> Check {
    let homes = codex_homes();
    let mut dirs = Vec::new();
    for home in &homes {
        let sessions = home.join("sessions");
        if sessions.is_dir() {
            dirs.push(sessions);
        } else if home.is_dir() {
            dirs.push(home.clone());
        }
    }
    if dirs.is_empty() {
        return Check::new(
            "codex data",
            Status::Warn,
            format!("not found (looked in {})", homes_display(&homes)),
        )
        .with_hint("Run Codex CLI once, or point CODEX_HOME at its data directory.");
    }
    let mut files = Vec::new();
    for dir in &dirs {
        turbotokens_adapter_common::collect_usage_files(dir, &mut files);
    }
    Check::new(
        "codex data",
        Status::Ok,
        format!("{} ({} JSONL files)", homes_display(&dirs), files.len()),
    )
}

fn codex_homes() -> Vec<PathBuf> {
    if let Ok(env_paths) = env::var("CODEX_HOME") {
        return env_paths
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    home::home_dir()
        .map(|home| vec![home.join(".codex")])
        .unwrap_or_default()
}

fn check_other_agents() -> Check {
    const CANDIDATES: &[(&str, &str)] = &[
        ("opencode", ".local/share/opencode"),
        ("amp", ".local/share/amp"),
        ("gemini", ".gemini"),
        ("copilot", ".copilot"),
        ("grok", ".grok"),
    ];
    let detected = home::home_dir()
        .map(|home| {
            CANDIDATES
                .iter()
                .filter(|(_, relative)| home.join(relative).exists())
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if detected.is_empty() {
        Check::new("other agents", Status::Info, "no other agents detected")
    } else {
        Check::new(
            "other agents",
            Status::Ok,
            format!(
                "{} other agents detected ({})",
                detected.len(),
                detected.join(", ")
            ),
        )
    }
}

fn check_cache() -> Check {
    if let Ok(value) = env::var("TURBOTOKENS_CACHE") {
        let value = value.trim();
        if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("false") || value == "0"
        {
            return Check::new(
                "parse cache",
                Status::Info,
                "disabled via TURBOTOKENS_CACHE",
            );
        }
    }
    let root = env::var("TURBOTOKENS_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::temp_dir().join("turbotokens-cache"));
    if !root.is_dir() {
        return Check::new(
            "parse cache",
            Status::Info,
            format!("{} not created yet", root.display()),
        )
        .with_hint("The cache is created on the first report run; nothing to do.");
    }
    let (files, bytes) = dir_stats(&root);
    Check::new(
        "parse cache",
        Status::Ok,
        format!(
            "{} ({} entries, {})",
            root.display(),
            files,
            format_bytes(bytes)
        ),
    )
}

#[cfg(unix)]
fn check_daemon() -> Check {
    let socket = env::temp_dir().join("turbotokens-daemon.sock");
    if !socket.exists() {
        return Check::new(
            "daemon",
            Status::Info,
            format!("not running (no socket at {})", socket.display()),
        )
        .with_hint("Optional: start with `turbotokens daemon start` for near-instant reports.");
    }
    match std::os::unix::net::UnixStream::connect(&socket) {
        Ok(_) => Check::new(
            "daemon",
            Status::Ok,
            format!("responding at {}", socket.display()),
        ),
        Err(_) => Check::new(
            "daemon",
            Status::Warn,
            format!("socket at {} is not responding", socket.display()),
        )
        .with_hint("Remove the stale socket and restart the daemon."),
    }
}

#[cfg(not(unix))]
fn check_daemon() -> Check {
    Check::new(
        "daemon",
        Status::Info,
        "socket check is not supported on this platform",
    )
}

fn check_config(shared: &SharedArgs) -> Check {
    let mut candidates = Vec::new();
    if let Some(path) = &shared.config {
        candidates.push(path.clone());
    }
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd.join(".turbotokens").join("turbotokens.json"));
    }
    for dir in claude_config_dirs() {
        candidates.push(dir.join("turbotokens.json"));
    }
    let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
        return Check::new(
            "config file",
            Status::Info,
            "no config file found; defaults in use",
        );
    };
    let valid = fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .is_some_and(|value| value.is_object());
    if valid {
        Check::new(
            "config file",
            Status::Ok,
            format!("{} (valid JSON)", path.display()),
        )
    } else {
        Check::new(
            "config file",
            Status::Warn,
            format!("{} is not valid JSON", path.display()),
        )
        .with_hint("Fix the JSON syntax or remove the file; it is currently ignored.")
    }
}

fn claude_config_dirs() -> Vec<PathBuf> {
    if let Ok(paths) = env::var("CLAUDE_CONFIG_DIR") {
        return paths
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    home::home_dir()
        .map(|home| vec![home.join(".config").join("claude"), home.join(".claude")])
        .unwrap_or_default()
}

fn check_pricing() -> Check {
    let models = PricingMap::load_embedded().model_count();
    if models > 0 {
        Check::new(
            "embedded pricing",
            Status::Ok,
            format!("loadable ({models} models)"),
        )
    } else {
        Check::new(
            "embedded pricing",
            Status::Fail,
            "pricing snapshot is empty",
        )
        .with_hint(
            "Rebuild with --features fetch-litellm-pricing or set TURBOTOKENS_PRICING_JSON_PATH.",
        )
    }
}

fn homes_display(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn dir_stats(root: &Path) -> (u64, u64) {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if metadata.is_file() {
                files += 1;
                bytes += metadata.len();
            }
        }
    }
    (files, bytes)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = UNITS[0];
    for next in &UNITS[1..] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    if unit == UNITS[0] {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {unit}")
    }
}

fn checks_json(checks: &[Check]) -> Value {
    json!({
        "version": env!("TURBOTOKENS_VERSION"),
        "ok": checks.iter().all(|check| check.status != Status::Fail),
        "checks": checks.iter().map(|check| json!({
            "name": check.name,
            "status": check.status.as_str(),
            "detail": check.detail,
            "hint": check.hint,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_output_has_expected_shape() {
        let checks = vec![
            Check::new("version", Status::Ok, "1.0.0"),
            Check::new("claude data", Status::Warn, "missing").with_hint("install it"),
        ];

        let value = checks_json(&checks);

        assert_eq!(value["version"], json!(env!("TURBOTOKENS_VERSION")));
        assert_eq!(value["ok"], json!(true));
        assert_eq!(value["checks"][0]["status"], json!("ok"));
        assert_eq!(value["checks"][1]["hint"], json!("install it"));
        assert!(value["checks"][0]["hint"].is_null());
    }

    #[test]
    fn json_ok_is_false_on_hard_failure() {
        let checks = vec![Check::new("embedded pricing", Status::Fail, "empty")];

        assert_eq!(checks_json(&checks)["ok"], json!(false));
    }

    #[test]
    fn collects_all_doctor_checks() {
        let checks = collect_checks(&SharedArgs::default());
        let names = checks.iter().map(|check| check.name).collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "version",
                "claude data",
                "codex data",
                "other agents",
                "parse cache",
                "daemon",
                "config file",
                "embedded pricing",
            ]
        );
    }

    #[test]
    fn formats_byte_sizes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024 + 1024 * 512), "3.5 MB");
    }
}
