//! Client side of the resident daemon protocol, plus the wire types shared
//! with the server (`daemon.rs`). Everything here fails soft: any error,
//! missing socket, or incompatible daemon falls back to the normal load path.

#[cfg(any(unix, test))]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::time::Duration;

#[cfg(any(unix, test))]
use serde::{Deserialize, Serialize};
#[cfg(any(unix, test))]
use serde_json::{Map, Value};

#[cfg(any(unix, test))]
use turbotokens_cli::PricingOverride;

use crate::{UsageSummary, cli::SharedArgs};

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
    try_daily_from_socket(&socket_path(), shared, project, group_by_project)
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
pub(crate) fn socket_path() -> std::path::PathBuf {
    std::env::temp_dir().join("turbotokens-daemon.sock")
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
    let request = DaemonRequest {
        command: "daily".to_string(),
        project: project.map(str::to_string),
        group_by_project,
    };
    let response = request_response(socket, &request, DAEMON_READ_TIMEOUT).ok()?;
    if !response.ok {
        return None;
    }
    if !response.started_with?.compatible_with(shared) {
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

    use std::os::unix::net::UnixStream;

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
}

#[cfg(any(unix, test))]
impl DaemonRequest {
    pub(crate) fn ping() -> Self {
        Self {
            command: "ping".to_string(),
            project: None,
            group_by_project: false,
        }
    }

    pub(crate) fn shutdown() -> Self {
        Self {
            command: "shutdown".to_string(),
            project: None,
            group_by_project: false,
        }
    }
}

#[cfg(any(unix, test))]
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DaemonResponse {
    pub(crate) ok: bool,
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

/// The load-affecting args the daemon was started with. Rows are only
/// interchangeable with a direct load when every one of these matches.
#[cfg(any(unix, test))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StartedWith {
    pub(crate) timezone: Option<String>,
    pub(crate) mode: String,
    pub(crate) offline: bool,
    #[serde(rename = "pricingOverrides", default)]
    pub(crate) pricing_overrides: BTreeMap<String, Value>,
}

#[cfg(any(unix, test))]
impl StartedWith {
    pub(crate) fn from_shared(shared: &SharedArgs) -> Self {
        Self {
            timezone: shared.timezone.clone(),
            mode: cost_mode_name(shared.mode).to_string(),
            offline: shared.offline,
            pricing_overrides: pricing_overrides_json(shared),
        }
    }

    pub(crate) fn compatible_with(&self, shared: &SharedArgs) -> bool {
        self.timezone == shared.timezone
            && self.offline == shared.offline
            && modes_compatible(shared.mode, &self.mode)
            && self.pricing_overrides == pricing_overrides_json(shared)
    }
}

#[cfg(any(unix, test))]
pub(crate) fn cost_mode_name(mode: crate::cli::CostMode) -> &'static str {
    match mode {
        crate::cli::CostMode::Auto => "auto",
        crate::cli::CostMode::Calculate => "calculate",
        crate::cli::CostMode::Display => "display",
    }
}

/// Auto and Calculate both price from the same cost data; Display skips
/// pricing entirely and only matches Display.
#[cfg(any(unix, test))]
fn modes_compatible(client: crate::cli::CostMode, daemon_mode: &str) -> bool {
    match (client, daemon_mode) {
        (crate::cli::CostMode::Display, "display") => true,
        (crate::cli::CostMode::Display, _) => false,
        (_, "display") => false,
        (crate::cli::CostMode::Auto | crate::cli::CostMode::Calculate, "auto" | "calculate") => {
            true
        }
        _ => false,
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
        assert!(StartedWith::from_shared(&shared).compatible_with(&shared));
    }

    #[test]
    fn treats_auto_and_calculate_as_compatible_but_not_display() {
        let auto = StartedWith::from_shared(&shared_with(CostMode::Auto, true, None));
        assert!(auto.compatible_with(&shared_with(CostMode::Calculate, true, None)));
        assert!(!auto.compatible_with(&shared_with(CostMode::Display, true, None)));

        let display = StartedWith::from_shared(&shared_with(CostMode::Display, true, None));
        assert!(display.compatible_with(&shared_with(CostMode::Display, true, None)));
        assert!(!display.compatible_with(&shared_with(CostMode::Auto, true, None)));
    }

    #[test]
    fn rejects_offline_timezone_and_override_mismatches() {
        let daemon = StartedWith::from_shared(&shared_with(CostMode::Auto, true, Some("UTC")));
        assert!(!daemon.compatible_with(&shared_with(CostMode::Auto, false, Some("UTC"))));
        assert!(!daemon.compatible_with(&shared_with(CostMode::Auto, true, None)));
        assert!(!daemon.compatible_with(&shared_with(CostMode::Auto, true, Some("Asia/Tokyo"))));

        let mut overridden = shared_with(CostMode::Auto, true, Some("UTC"));
        overridden.pricing_overrides.insert(
            "model".to_string(),
            PricingOverride {
                input_cost_per_token: Some(1e-6),
                ..Default::default()
            },
        );
        assert!(!daemon.compatible_with(&overridden));
        assert!(StartedWith::from_shared(&overridden).compatible_with(&overridden));
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
        let started_with = StartedWith::from_shared(&shared);
        let json = serde_json::to_string(&started_with).unwrap();
        let decoded: StartedWith = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, started_with);
    }
}
