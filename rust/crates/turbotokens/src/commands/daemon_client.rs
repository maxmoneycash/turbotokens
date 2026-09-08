//! Client side of the resident daemon protocol, plus the wire types shared
//! with the server (`daemon.rs`). Everything here fails soft: any error,
//! missing socket, or incompatible daemon falls back to the normal load path.

#[cfg(any(unix, test))]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::Path;
#[cfg(any(unix, test))]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

#[cfg(any(unix, test))]
use serde::{Deserialize, Serialize};
#[cfg(any(unix, test))]
use serde_json::{Map, Value};

#[cfg(any(unix, test))]
use turbotokens_cli::PricingOverride;

use crate::{UsageSummary, cli::SharedArgs};

#[cfg(any(unix, test))]
pub(crate) const DAEMON_PROTOCOL: &str = "turbotokens-daemon-v1";

/// Reads may legitimately wait behind an in-flight query or poll; anything
/// longer means the daemon is wedged and the caller should fall back to
/// loading directly.
#[cfg(unix)]
pub(crate) const DAEMON_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Serves daily rows from the resident daemon when one is running with
/// compatible load-affecting args. Returns `None` on any error or mismatch —
/// the caller then falls back to the normal load path.
#[cfg(unix)]
pub(crate) fn try_daily_from_daemon(
    shared: &SharedArgs,
    project: Option<&str>,
    group_by_project: bool,
) -> Option<Vec<UsageSummary>> {
    try_daily_from_socket(&socket_path().ok()?, shared, project, group_by_project)
}

#[cfg(not(unix))]
pub(crate) fn try_daily_from_daemon(
    shared: &SharedArgs,
    project: Option<&str>,
    group_by_project: bool,
) -> Option<Vec<UsageSummary>> {
    let _ = (shared, project, group_by_project);
    None
}

#[cfg(unix)]
pub(crate) fn socket_path() -> std::io::Result<std::path::PathBuf> {
    Ok(super::daemon_paths::DaemonPaths::resolve(false)?.socket)
}

#[cfg(unix)]
pub(crate) fn try_daily_from_socket(
    socket: &std::path::Path,
    shared: &SharedArgs,
    project: Option<&str>,
    group_by_project: bool,
) -> Option<Vec<UsageSummary>> {
    // No socket file means no daemon: the cheapest possible miss.
    if !socket.exists() {
        return None;
    }
    let source_paths = turbotokens_adapter_claude::claude_paths().ok()?;
    try_daily_from_socket_with_paths(socket, shared, project, group_by_project, &source_paths)
}

#[cfg(unix)]
pub(crate) fn try_daily_from_socket_with_paths(
    socket: &Path,
    shared: &SharedArgs,
    project: Option<&str>,
    group_by_project: bool,
    source_paths: &[PathBuf],
) -> Option<Vec<UsageSummary>> {
    let request = DaemonRequest {
        command: "daily".to_string(),
        project: project.map(str::to_string),
        group_by_project,
        expected_pid: None,
    };
    let response = request_response(socket, &request, DAEMON_READ_TIMEOUT).ok()?;
    if !response.is_current() {
        return None;
    }
    if !response.started_with?.compatible_with(shared, source_paths) {
        return None;
    }
    response.rows
}

/// Sends one newline-delimited JSON request and reads the JSON response line.
#[cfg(unix)]
pub(crate) fn request_response(
    socket: &std::path::Path,
    request: &DaemonRequest,
    timeout: Duration,
) -> std::io::Result<DaemonResponse> {
    use std::io::{BufRead, BufReader, Write};

    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::net::UnixStream;

    if !std::fs::symlink_metadata(socket)?.file_type().is_socket() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "daemon path must be a Unix socket, without links",
        ));
    }

    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut payload = serde_json::to_vec(request)?;
    payload.push(b'\n');
    stream.write_all(&payload)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    if line.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "daemon closed the connection without a response",
        ));
    }
    Ok(serde_json::from_str(&line)?)
}

#[cfg(any(unix, test))]
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DaemonRequest {
    pub(crate) command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project: Option<String>,
    #[serde(rename = "groupByProject", default)]
    pub(crate) group_by_project: bool,
    #[serde(
        rename = "expectedPid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) expected_pid: Option<u32>,
}

#[cfg(any(unix, test))]
impl DaemonRequest {
    pub(crate) fn ping() -> Self {
        Self {
            command: "ping".to_string(),
            project: None,
            group_by_project: false,
            expected_pid: None,
        }
    }

    pub(crate) fn shutdown(pid: u32) -> Self {
        Self {
            command: "shutdown".to_string(),
            project: None,
            group_by_project: false,
            expected_pid: Some(pid),
        }
    }
}

#[cfg(any(unix, test))]
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DaemonResponse {
    pub(crate) ok: bool,
    #[serde(default)]
    pub(crate) protocol: Option<String>,
    #[serde(default)]
    pub(crate) pid: Option<u32>,
    #[serde(rename = "uptimeMs", default)]
    pub(crate) uptime_ms: Option<u64>,
    #[serde(default)]
    pub(crate) files: Option<usize>,
    #[serde(default)]
    pub(crate) entries: Option<usize>,
    #[serde(rename = "startedWith", default)]
    pub(crate) started_with: Option<StartedWith>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rows: Option<Vec<UsageSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[cfg(any(unix, test))]
impl DaemonResponse {
    pub(crate) fn is_current(&self) -> bool {
        self.ok && self.protocol.as_deref() == Some(DAEMON_PROTOCOL)
    }
}

/// The load-affecting options and directories represented by the daemon.
/// Missing source identity from an older daemon requires a direct load.
#[cfg(any(unix, test))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StartedWith {
    pub(crate) timezone: Option<String>,
    #[serde(rename = "resolvedTimezone", default)]
    pub(crate) resolved_timezone: Option<String>,
    pub(crate) mode: String,
    pub(crate) offline: bool,
    #[serde(rename = "pricingOverrides", default)]
    pub(crate) pricing_overrides: BTreeMap<String, Value>,
    #[serde(rename = "sourcePaths", default)]
    pub(crate) source_paths: Option<Vec<PathBuf>>,
}

#[cfg(any(unix, test))]
impl StartedWith {
    pub(crate) fn from_shared(shared: &SharedArgs, source_paths: &[PathBuf]) -> Self {
        Self {
            timezone: shared.timezone.clone(),
            resolved_timezone: turbotokens_core::date_utils::timezone_identity(
                shared.timezone.as_deref(),
            ),
            mode: cost_mode_name(shared.mode).to_string(),
            offline: shared.offline,
            pricing_overrides: pricing_overrides_json(shared),
            source_paths: normalized_source_paths(source_paths),
        }
    }

    pub(crate) fn compatible_with(&self, shared: &SharedArgs, source_paths: &[PathBuf]) -> bool {
        self.timezone == shared.timezone
            && self.resolved_timezone.is_some()
            && self.resolved_timezone
                == turbotokens_core::date_utils::timezone_identity(shared.timezone.as_deref())
            && self.offline == shared.offline
            && self.mode == cost_mode_name(shared.mode)
            && self.pricing_overrides == pricing_overrides_json(shared)
            && self.source_paths.is_some()
            && self.source_paths == normalized_source_paths(source_paths)
    }
}

#[cfg(any(unix, test))]
fn normalized_source_paths(paths: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let mut paths = paths
        .iter()
        .map(|path| path.canonicalize())
        .collect::<std::io::Result<Vec<_>>>()
        .ok()?;
    paths.sort();
    paths.dedup();
    Some(paths)
}

#[cfg(any(unix, test))]
pub(crate) fn cost_mode_name(mode: crate::cli::CostMode) -> &'static str {
    match mode {
        crate::cli::CostMode::Auto => "auto",
        crate::cli::CostMode::Calculate => "calculate",
        crate::cli::CostMode::Display => "display",
    }
}

#[cfg(any(unix, test))]
fn pricing_overrides_json(shared: &SharedArgs) -> BTreeMap<String, Value> {
    shared
        .pricing_overrides
        .iter()
        .map(|(model, override_)| (model.clone(), pricing_override_json(override_)))
        .collect()
}

#[cfg(any(unix, test))]
fn pricing_override_json(override_: &PricingOverride) -> Value {
    let mut map = Map::new();
    let mut insert = |name: &str, value: Option<f64>| {
        if let Some(value) = value {
            map.insert(name.to_string(), Value::from(value));
        }
    };
    insert("inputCostPerToken", override_.input_cost_per_token);
    insert("outputCostPerToken", override_.output_cost_per_token);
    insert(
        "cacheCreationInputTokenCost",
        override_.cache_creation_input_token_cost,
    );
    insert(
        "cacheReadInputTokenCost",
        override_.cache_read_input_token_cost,
    );
    insert(
        "inputCostPerTokenAbove200kTokens",
        override_.input_cost_per_token_above_200k_tokens,
    );
    insert(
        "outputCostPerTokenAbove200kTokens",
        override_.output_cost_per_token_above_200k_tokens,
    );
    insert(
        "cacheCreationInputTokenCostAbove200kTokens",
        override_.cache_creation_input_token_cost_above_200k_tokens,
    );
    insert(
        "cacheReadInputTokenCostAbove200kTokens",
        override_.cache_read_input_token_cost_above_200k_tokens,
    );
    insert("fastMultiplier", override_.fast_multiplier);
    if let Some(max_input_tokens) = override_.max_input_tokens {
        map.insert("maxInputTokens".to_string(), Value::from(max_input_tokens));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CostMode;

    fn shared_with(mode: CostMode, offline: bool, timezone: Option<&str>) -> SharedArgs {
        SharedArgs {
            mode,
            offline,
            timezone: timezone.map(str::to_string),
            ..SharedArgs::default()
        }
    }

    #[test]
    fn matches_identical_start_args() {
        let shared = shared_with(CostMode::Auto, true, Some("UTC"));
        assert!(StartedWith::from_shared(&shared, &[]).compatible_with(&shared, &[]));
    }

    #[test]
    fn cost_modes_require_exact_matches() {
        // Auto trusts costUSD when present; Calculate recomputes it. A single
        // recorded cost can therefore make every pair of modes disagree.
        let modes = [CostMode::Auto, CostMode::Calculate, CostMode::Display];
        for daemon_mode in modes {
            let daemon =
                StartedWith::from_shared(&shared_with(daemon_mode, true, Some("UTC")), &[]);
            for client_mode in modes {
                let client = shared_with(client_mode, true, Some("UTC"));
                assert_eq!(
                    daemon.compatible_with(&client, &[]),
                    daemon_mode == client_mode
                );
            }
        }
    }

    #[test]
    fn omitted_timezone_still_requires_matching_resolved_identity() {
        let shared = shared_with(CostMode::Auto, true, None);
        let mut daemon = StartedWith::from_shared(&shared, &[]);
        assert_eq!(
            daemon.compatible_with(&shared, &[]),
            daemon.resolved_timezone.is_some()
        );
        let other_zone = if daemon.resolved_timezone.as_deref() == Some("UTC") {
            "America/Los_Angeles"
        } else {
            "UTC"
        };
        daemon.resolved_timezone =
            turbotokens_core::date_utils::timezone_identity(Some(other_zone));
        assert!(!daemon.compatible_with(&shared, &[]));
        daemon.resolved_timezone = None;
        assert!(!daemon.compatible_with(&shared, &[]));
    }

    #[test]
    fn rejects_legacy_daemons_without_effective_timezone_identity() {
        let shared = shared_with(CostMode::Display, true, Some("UTC"));
        let legacy: StartedWith = serde_json::from_str(
            r#"{"timezone":"UTC","mode":"display","offline":true,"pricingOverrides":{},"sourcePaths":[]}"#,
        ).unwrap();
        assert!(!legacy.compatible_with(&shared, &[]));
    }

    #[test]
    fn rejects_offline_timezone_and_override_mismatches() {
        let daemon = StartedWith::from_shared(&shared_with(CostMode::Auto, true, Some("UTC")), &[]);
        assert!(!daemon.compatible_with(&shared_with(CostMode::Auto, false, Some("UTC")), &[]));
        assert!(!daemon.compatible_with(&shared_with(CostMode::Auto, true, None), &[]));
        assert!(
            !daemon.compatible_with(&shared_with(CostMode::Auto, true, Some("Asia/Tokyo")), &[])
        );

        let mut overridden = shared_with(CostMode::Auto, true, Some("UTC"));
        overridden.pricing_overrides.insert(
            "model".to_string(),
            PricingOverride {
                input_cost_per_token: Some(1e-6),
                ..Default::default()
            },
        );
        assert!(!daemon.compatible_with(&overridden, &[]));
        assert!(StartedWith::from_shared(&overridden, &[]).compatible_with(&overridden, &[]));
    }

    #[test]
    fn serializes_started_with_stably_for_wire_comparison() {
        let mut shared = shared_with(CostMode::Calculate, true, None);
        shared.pricing_overrides.insert(
            "model".to_string(),
            PricingOverride {
                input_cost_per_token: Some(1.5e-6),
                max_input_tokens: Some(1_000_000),
                ..Default::default()
            },
        );
        let started_with = StartedWith::from_shared(&shared, &[]);
        let json = serde_json::to_string(&started_with).unwrap();
        let decoded: StartedWith = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, started_with);
    }

    #[test]
    fn rejects_different_source_directories() {
        let fixture = turbotokens_test_support::fs_fixture!({
            "source-a/projects/.keep": "",
            "source-b/projects/.keep": "",
        });
        let shared = shared_with(CostMode::Display, true, Some("UTC"));
        let source_a = fixture.path("source-a");
        let source_b = fixture.path("source-b");
        let daemon = StartedWith::from_shared(&shared, std::slice::from_ref(&source_a));

        assert!(daemon.compatible_with(&shared, std::slice::from_ref(&source_a)));
        assert!(!daemon.compatible_with(&shared, &[source_b]));
        assert!(daemon.compatible_with(&shared, &[source_a.join(".")]));
    }

    #[test]
    fn rejects_legacy_daemons_without_source_identity() {
        let shared = shared_with(CostMode::Display, true, Some("UTC"));
        let legacy: StartedWith = serde_json::from_str(
            r#"{"timezone":"UTC","mode":"display","offline":true,"pricingOverrides":{}}"#,
        )
        .unwrap();

        assert!(!legacy.compatible_with(&shared, &[]));
    }
}
