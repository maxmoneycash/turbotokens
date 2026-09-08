//! `turbotokens daemon`: a resident index process that keeps the daily report
//! hot. `run` serves newline-delimited JSON over a unix socket; `start`,
//! `stop`, and `status` manage that server from the command line.

use crate::{Result, cli::DaemonArgs, cli_error};

#[cfg(unix)]
use crate::cli::DaemonAction;

pub(crate) fn run(args: DaemonArgs) -> Result<()> {
    #[cfg(unix)]
    {
        match args.action {
            DaemonAction::Run => run_server(&args),
            DaemonAction::Start => start_server(),
            DaemonAction::Stop => stop_server(),
            DaemonAction::Status => status_server(),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        Err(cli_error(
            "turbotokens daemon mode is not supported on Windows yet",
        ))
    }
}

#[cfg(unix)]
use std::{
    env,
    ffi::OsString,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use turbotokens_adapter_claude::ResidentIndex;

#[cfg(unix)]
use super::daemon_client::{
    DAEMON_PROTOCOL, DAEMON_READ_TIMEOUT, DaemonRequest, DaemonResponse, StartedWith,
    request_response, socket_path,
};
#[cfg(unix)]
use super::daemon_paths::{DaemonPaths, PidFile, bind_socket, read_pid};

/// Server-side request read timeout; a wedged client must not stall the
/// poll loop.
#[cfg(unix)]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(unix)]
fn run_server(args: &DaemonArgs) -> Result<()> {
    let paths = DaemonPaths::resolve(true)?;
    let socket = paths.socket;
    if socket.exists()
        && let Ok(response) = request_response(&socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT)
        && response.is_current()
    {
        return Err(cli_error(format!(
            "turbotokens daemon is already running (pid {})",
            response.pid.unwrap_or_default()
        )));
    }
    let mut index = ResidentIndex::new(&args.shared)?;
    index.seed();
    eprintln!(
        "turbotokens daemon listening on {} (pid {}, {} files, {} entries)",
        socket.display(),
        std::process::id(),
        index.files_watched(),
        index.entries_indexed(),
    );
    serve(
        &socket,
        &paths.pid,
        &args.shared,
        args.interval_ms,
        &mut index,
        Instant::now(),
    )
}

/// Foreground serve loop: a blocking acceptor thread hands connections to
/// the main loop, which answers queries immediately and rebuilds the index
/// incrementally every `interval_ms`. Exits on a shutdown request.
#[cfg(unix)]
pub(crate) fn serve(
    socket: &Path,
    pid_file: &Path,
    shared: &crate::cli::SharedArgs,
    interval_ms: u64,
    index: &mut ResidentIndex,
    started: Instant,
) -> Result<()> {
    let mut pid_file = PidFile::acquire(pid_file)?;
    let (listener, _socket_file) = bind_socket(socket)?;
    pid_file.publish()?;

    let (connections, incoming) = std::sync::mpsc::channel::<UnixStream>();
    let acceptor = thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if connections.send(stream).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let started_with = StartedWith::from_shared(shared, index.source_paths());
    let interval = Duration::from_millis(interval_ms.max(1));
    let mut next_poll = Instant::now() + interval;
    let mut shutdown = false;
    while !shutdown {
        // Wake instantly for a connection; otherwise wake for the poll tick.
        let wait = next_poll.saturating_duration_since(Instant::now());
        match incoming.recv_timeout(wait) {
            Ok(stream) => {
                shutdown |=
                    handle_connection(stream, index, &started_with, started).unwrap_or(false);
                while let Ok(stream) = incoming.try_recv() {
                    shutdown |=
                        handle_connection(stream, index, &started_with, started).unwrap_or(false);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if shutdown {
            break;
        }
        let now = Instant::now();
        if now >= next_poll {
            index.poll();
            next_poll = now + interval;
        }
    }
    drop(incoming);
    // Wake the acceptor so it observes the closed channel and exits.
    let _ = UnixStream::connect(socket);
    let _ = acceptor.join();
    Ok(())
}

/// Handles one query connection. Returns `true` when the server should shut
/// down after answering.
#[cfg(unix)]
fn handle_connection(
    stream: UnixStream,
    index: &mut ResidentIndex,
    started_with: &StartedWith,
    started: Instant,
) -> std::io::Result<bool> {
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let request: DaemonRequest = match serde_json::from_str(line.trim()) {
        Ok(request) => request,
        Err(error) => {
            let mut response = empty_response(started_with, started, index);
            response.ok = false;
            response.error = Some(format!("invalid request: {error}"));
            write_response(reader.get_mut(), &response)?;
            return Ok(false);
        }
    };
    match request.command.as_str() {
        "ping" => {
            let response = empty_response(started_with, started, index);
            write_response(reader.get_mut(), &response)?;
            Ok(false)
        }
        "daily" => {
            let rows = index.daily_rows(request.project.as_deref(), request.group_by_project);
            let mut response = empty_response(started_with, started, index);
            response.rows = Some(rows);
            write_response(reader.get_mut(), &response)?;
            Ok(false)
        }
        "shutdown" => {
            let mut response = empty_response(started_with, started, index);
            if request.expected_pid != Some(std::process::id()) {
                response.ok = false;
                response.error = Some("shutdown requires the matching daemon PID".to_string());
            }
            write_response(reader.get_mut(), &response)?;
            Ok(response.ok)
        }
        command => {
            let mut response = empty_response(started_with, started, index);
            response.ok = false;
            response.error = Some(format!("unknown command: {command}"));
            write_response(reader.get_mut(), &response)?;
            Ok(false)
        }
    }
}

#[cfg(unix)]
fn empty_response(
    started_with: &StartedWith,
    started: Instant,
    index: &ResidentIndex,
) -> DaemonResponse {
    DaemonResponse {
        ok: true,
        protocol: Some(DAEMON_PROTOCOL.to_string()),
        pid: Some(std::process::id()),
        uptime_ms: Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
        files: Some(index.files_watched()),
        entries: Some(index.entries_indexed()),
        started_with: Some(started_with.clone()),
        rows: None,
        error: None,
    }
}

#[cfg(unix)]
fn write_response(stream: &mut UnixStream, response: &DaemonResponse) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

#[cfg(unix)]
fn start_server() -> Result<()> {
    let socket = DaemonPaths::resolve(true)?.socket;
    if socket.exists()
        && let Ok(response) = request_response(&socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT)
        && response.is_current()
    {
        println!(
            "turbotokens daemon is already running (pid {})",
            response.pid.unwrap_or_default()
        );
        return Ok(());
    }

    let mut child = ProcessCommand::new(env::current_exe()?)
        .args(run_args()?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let pid = child.id();

    // The initial scan can take a moment on large logs; wait for the server
    // to answer before reporting success.
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(50));
        if let Ok(response) = request_response(&socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT)
            && response.is_current()
            && response.pid == Some(pid)
        {
            println!("turbotokens daemon started (pid {pid})");
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(cli_error(format!(
                "daemon exited ({status}); run `turbotokens daemon run` to see the startup error"
            )));
        }
    }
    println!("turbotokens daemon spawned (pid {pid}) but is not answering yet");
    Ok(())
}

/// Rewrites this invocation's argv from `daemon start` to `daemon run`,
/// preserving every other flag verbatim.
#[cfg(unix)]
fn run_args() -> Result<Vec<OsString>> {
    let mut args = env::args_os().skip(1).collect::<Vec<_>>();
    let daemon_index = args
        .iter()
        .position(|arg| arg == "daemon")
        .ok_or_else(|| cli_error("cannot reconstruct daemon arguments"))?;
    let action_index = daemon_index + 1;
    if args.get(action_index).is_some_and(|arg| arg == "start") {
        args[action_index] = OsString::from("run");
    }
    Ok(args)
}

#[cfg(unix)]
fn stop_server() -> Result<()> {
    let paths = DaemonPaths::resolve(false)?;
    stop_matching_server(&paths.socket, &paths.pid)
}

#[cfg(unix)]
fn stop_matching_server(socket: &Path, pid_file: &Path) -> Result<()> {
    // A PID file alone is never authority to signal a process. Verify both
    // ends of this private IPC endpoint, then ask that exact daemon to exit.
    let pid = read_pid(pid_file)?;
    let ping = request_response(socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT)?;
    if !ping.is_current() || ping.pid != Some(pid) {
        return Err(cli_error(
            "daemon socket and PID record do not match; refusing to stop",
        ));
    }
    let response = request_response(socket, &DaemonRequest::shutdown(pid), DAEMON_READ_TIMEOUT)?;
    if !response.is_current() || response.pid != Some(pid) {
        return Err(cli_error("daemon did not confirm shutdown"));
    }
    println!("turbotokens daemon stopped (pid {pid})");
    Ok(())
}

#[cfg(unix)]
fn status_server() -> Result<()> {
    let socket = socket_path()?;
    let response = if socket.exists() {
        request_response(&socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT).ok()
    } else {
        None
    };
    let Some(response) = response.filter(DaemonResponse::is_current) else {
        return Err(cli_error("turbotokens daemon is not running"));
    };
    println!("turbotokens daemon running");
    println!("  pid: {}", response.pid.unwrap_or_default());
    println!(
        "  uptime: {}",
        format_uptime(response.uptime_ms.unwrap_or(0))
    );
    println!("  files watched: {}", response.files.unwrap_or_default());
    println!(
        "  entries indexed: {}",
        response.entries.unwrap_or_default()
    );
    if let Some(started_with) = &response.started_with {
        println!(
            "  started with: mode={}, offline={}, timezone={}, pricing overrides={}",
            started_with.mode,
            started_with.offline,
            started_with.timezone.as_deref().unwrap_or("system"),
            started_with.pricing_overrides.len(),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn format_uptime(millis: u64) -> String {
    let seconds = millis / 1000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::cli::CostMode;
    use std::fs;
    use turbotokens_test_support::fs_fixture;

    fn usage_line(message_id: &str, output_tokens: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-07-28T10:00:00.000Z","version":"1.2.3","sessionId":"s","message":{{"id":"{message_id}","model":"claude-sonnet-4-20250514","usage":{{"input_tokens":100,"output_tokens":{output_tokens},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}},"requestId":"req-{message_id}","costUSD":0.01}}"#
        )
    }

    fn shared() -> crate::cli::SharedArgs {
        crate::cli::SharedArgs {
            mode: CostMode::Display,
            offline: true,
            timezone: Some("UTC".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn serves_daily_rows_tracks_appends_and_shuts_down() {
        let fixture = fs_fixture!({
            "projects/p/s.jsonl": format!("{}\n", usage_line("msg-1", 10)),
        });
        let socket = fixture.path("daemon.sock");
        let pid_file = fixture.path("daemon.pid");
        let shared = shared();
        let mut index = ResidentIndex::with_paths(&shared, vec![fixture.root().to_path_buf()]);
        index.seed();

        let server = {
            let socket = socket.clone();
            let pid_file = pid_file.clone();
            let shared = shared.clone();
            thread::spawn(move || {
                serve(&socket, &pid_file, &shared, 50, &mut index, Instant::now())
            })
        };
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        // Warmup query, then measure.
        let rows = super::super::daemon_client::try_daily_from_socket_with_paths(
            &socket,
            &shared,
            None,
            false,
            &[fixture.root().to_path_buf()],
        )
        .expect("daemon serves rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date.as_deref(), Some("2026-07-28"));
        assert_eq!(rows[0].input_tokens, 100);
        assert_eq!(rows[0].output_tokens, 10);
        assert!((rows[0].total_cost - 0.01).abs() < 1e-9);

        let mut samples = Vec::new();
        for _ in 0..25 {
            let start = Instant::now();
            super::super::daemon_client::try_daily_from_socket_with_paths(
                &socket,
                &shared,
                None,
                false,
                &[fixture.root().to_path_buf()],
            )
            .expect("daemon serves rows");
            samples.push(start.elapsed());
        }
        samples.sort();
        let median = samples[samples.len() / 2];
        eprintln!(
            "daemon query latency: min={:?} median={:?} p95={:?} max={:?}",
            samples[0],
            median,
            samples[samples.len() * 95 / 100],
            samples[samples.len() - 1],
        );
        assert!(
            median < Duration::from_millis(10),
            "median query latency {median:?} exceeds 10ms"
        );

        // Incompatible args fall back to the direct load path.
        let mismatched = crate::cli::SharedArgs {
            timezone: Some("Asia/Tokyo".to_string()),
            ..shared.clone()
        };
        assert!(
            super::super::daemon_client::try_daily_from_socket_with_paths(
                &socket,
                &mismatched,
                None,
                false,
                &[fixture.root().to_path_buf()]
            )
            .is_none()
        );

        let other_source =
            fs_fixture!({ "projects/other/session.jsonl": usage_line("other", 999) });
        assert!(
            super::super::daemon_client::try_daily_from_socket_with_paths(
                &socket,
                &shared,
                None,
                false,
                &[other_source.root().to_path_buf()],
            )
            .is_none(),
            "a daemon indexing a different Claude directory must be bypassed",
        );

        // Appended lines show up within a couple of poll intervals.
        use std::io::Write as _;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(fixture.path("projects/p/s.jsonl"))
            .unwrap();
        writeln!(file, "{}", usage_line("msg-2", 20)).unwrap();
        writeln!(file, "{}", usage_line("msg-3", 30)).unwrap();
        drop(file);
        thread::sleep(Duration::from_millis(250));

        let rows = super::super::daemon_client::try_daily_from_socket_with_paths(
            &socket,
            &shared,
            None,
            false,
            &[fixture.root().to_path_buf()],
        )
        .expect("daemon serves rows");
        assert_eq!(rows[0].input_tokens, 300);
        assert_eq!(rows[0].output_tokens, 60);
        assert!((rows[0].total_cost - 0.03).abs() < 1e-9);

        let grouped = super::super::daemon_client::try_daily_from_socket_with_paths(
            &socket,
            &shared,
            None,
            true,
            &[fixture.root().to_path_buf()],
        )
        .expect("daemon serves grouped rows");
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].project.as_deref(), Some("p"));

        let ping = request_response(&socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT)
            .expect("ping answers");
        assert!(ping.ok);
        assert!(ping.entries.unwrap_or_default() >= 3);

        let refused = request_response(&socket, &DaemonRequest::shutdown(0), DAEMON_READ_TIMEOUT)
            .expect("mismatched shutdown answers");
        assert!(!refused.ok);
        assert!(
            request_response(&socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT)
                .unwrap()
                .is_current()
        );

        // An unrelated PID record must never authorize shutdown or a signal.
        fs::write(&pid_file, "1").unwrap();
        assert!(stop_matching_server(&socket, &pid_file).is_err());
        assert!(
            request_response(&socket, &DaemonRequest::ping(), DAEMON_READ_TIMEOUT)
                .unwrap()
                .is_current()
        );
        fs::write(&pid_file, std::process::id().to_string()).unwrap();
        stop_matching_server(&socket, &pid_file).expect("matching daemon stops");
        server.join().expect("server thread exits").unwrap();
        assert!(!socket.exists());
        assert!(!pid_file.exists());
    }
}
