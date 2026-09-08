//! Private Unix IPC paths. Report lookups never create directories, and legacy
//! files in the shared temporary directory are neither read nor removed.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
};

pub(crate) struct DaemonPaths {
    pub(crate) socket: PathBuf,
    pub(crate) pid: PathBuf,
}

impl DaemonPaths {
    pub(crate) fn resolve(create: bool) -> io::Result<Self> {
        Self::from_home(turbotokens_core::home::home_dir().as_deref(), create)
    }

    fn from_home(home: Option<&Path>, create: bool) -> io::Result<Self> {
        let home = home.filter(|path| path.is_absolute()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "daemon requires an absolute HOME directory",
            )
        })?;
        let home_metadata = home.metadata()?;
        if !home_metadata.is_dir() {
            return Err(unsafe_path(home, "HOME is not a directory"));
        }
        let root = home.join(".turbotokens");
        let socket = root.join("daemon/daemon.sock");
        std::os::unix::net::SocketAddr::from_pathname(&socket).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "unsupported daemon socket path {}: {error}",
                    socket.display()
                ),
            )
        })?;
        for (directory, forbidden_permissions, expectation) in [
            (
                &root,
                0o022,
                "expected an owned directory without group or other write access",
            ),
            (
                &root.join("daemon"),
                0o077,
                "expected a private, owned directory (0700)",
            ),
        ] {
            if create {
                match fs::DirBuilder::new().mode(0o700).create(directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            let metadata = fs::symlink_metadata(directory)?;
            if !metadata.is_dir()
                || metadata.uid() != home_metadata.uid()
                || metadata.mode() & forbidden_permissions != 0
            {
                return Err(unsafe_path(directory, expectation));
            }
        }
        Ok(Self {
            socket,
            pid: root.join("daemon/daemon.pid"),
        })
    }
}

fn unsafe_path(path: &Path, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{}: {reason}", path.display()),
    )
}

fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}

fn validate_pid_file(path: &Path, file: &File) -> io::Result<fs::Metadata> {
    let metadata = file.metadata()?;
    let path_metadata = fs::symlink_metadata(path)?;
    if !path_metadata.is_file()
        || !same_file(&metadata, &path_metadata)
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || metadata.uid()
            != path
                .parent()
                .ok_or_else(|| unsafe_path(path, "missing parent"))?
                .metadata()?
                .uid()
    {
        return Err(unsafe_path(
            path,
            "expected a private, owned PID file (0600), without links",
        ));
    }
    Ok(metadata)
}

/// Keep an OS file lock for the daemon's lifetime. A crash releases the lock;
/// the next start can reuse the stale file without trusting its old PID.
pub(crate) struct PidFile {
    path: PathBuf,
    file: File,
    remove_on_drop: bool,
}

impl PidFile {
    pub(crate) fn acquire(path: &Path) -> io::Result<Self> {
        let (file, created) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(file) => (file, true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // Reject symlinks before opening; recheck the open file's inode
                // against the path before any write so a changed path also fails.
                if !fs::symlink_metadata(path)?.is_file() {
                    return Err(unsafe_path(path, "PID path must be a regular file"));
                }
                (OpenOptions::new().read(true).write(true).open(path)?, false)
            }
            Err(error) => return Err(error),
        };
        validate_pid_file(path, &file)?;
        file.try_lock().map_err(|error| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("daemon PID file is locked at {}: {error}", path.display()),
            )
        })?;
        validate_pid_file(path, &file)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            remove_on_drop: created,
        })
    }

    pub(crate) fn publish(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        write!(self.file, "{}", std::process::id())?;
        self.file.flush()?;
        self.remove_on_drop = true;
        Ok(())
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if self.remove_on_drop && validate_pid_file(&self.path, &self.file).is_ok() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(crate) fn read_pid(path: &Path) -> io::Result<u32> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(unsafe_path(path, "PID path must be a regular file"));
    }
    let file = File::open(path)?;
    if validate_pid_file(path, &file)?.len() > 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid daemon PID record",
        ));
    }
    let mut content = String::new();
    file.take(32).read_to_string(&mut content)?;
    content
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid daemon PID record"))
}

/// Called only while holding the PID lock. Never unlink a responding socket,
/// a symlink, or an unrelated file; only a refused stale socket can be replaced.
pub(crate) fn bind_socket(path: &Path) -> io::Result<(UnixListener, SocketFile)> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() {
                return Err(unsafe_path(path, "daemon socket path is not a socket"));
            }
            match UnixStream::connect(path) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "daemon socket is already accepting connections",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    if !same_file(&metadata, &fs::symlink_metadata(path)?) {
                        return Err(unsafe_path(path, "daemon socket changed during startup"));
                    }
                    fs::remove_file(path)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot bind daemon socket at {}: {error}", path.display()),
        )
    })?;
    let guard = SocketFile {
        path: path.to_path_buf(),
        metadata: fs::symlink_metadata(path)?,
    };
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok((listener, guard))
}

pub(crate) struct SocketFile {
    path: PathBuf,
    metadata: fs::Metadata,
}

impl Drop for SocketFile {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| same_file(&metadata, &self.metadata))
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use turbotokens_test_support::fs_fixture;

    #[test]
    fn lookups_are_read_only_and_start_creates_private_directories() {
        let fixture = fs_fixture!({ "home/.keep": "" });
        let home = fixture.path("home");
        assert!(DaemonPaths::from_home(Some(&home), false).is_err());
        assert!(!home.join(".turbotokens").exists());
        let paths = DaemonPaths::from_home(Some(&home), true).unwrap();
        assert_eq!(paths.socket, home.join(".turbotokens/daemon/daemon.sock"));
        assert_eq!(
            paths.socket.parent().unwrap().metadata().unwrap().mode() & 0o777,
            0o700
        );
        assert!(DaemonPaths::from_home(Some(&home), false).is_ok());
        assert!(DaemonPaths::from_home(None, true).is_err());
        assert!(DaemonPaths::from_home(Some(Path::new("relative")), true).is_err());
    }

    #[test]
    fn refuses_shared_and_symlinked_directories_without_changing_them() {
        let fixture = fs_fixture!({ "home/.turbotokens/.keep": "", "target/.keep": "" });
        let home = fixture.path("home");
        let root = home.join(".turbotokens");
        for mode in [0o775, 0o777] {
            fs::set_permissions(&root, fs::Permissions::from_mode(mode)).unwrap();
            assert!(DaemonPaths::from_home(Some(&home), true).is_err());
            assert_eq!(root.metadata().unwrap().mode() & 0o777, mode);
        }
        fs::remove_dir_all(&root).unwrap();
        symlink(fixture.path("target"), &root).unwrap();
        assert!(DaemonPaths::from_home(Some(&home), true).is_err());
        assert!(!fixture.path("target/daemon").exists());
    }

    #[test]
    fn accepts_an_owned_readable_config_directory_but_keeps_ipc_private() {
        let fixture = fs_fixture!({ "home/.turbotokens/config.json": "{}" });
        let home = fixture.path("home");
        let root = home.join(".turbotokens");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let paths = DaemonPaths::from_home(Some(&home), true).unwrap();
        let daemon = paths.socket.parent().unwrap();
        assert_eq!(root.metadata().unwrap().mode() & 0o777, 0o755);
        assert_eq!(daemon.metadata().unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::read_to_string(root.join("config.json")).unwrap(), "{}");
        assert!(DaemonPaths::from_home(Some(&home), false).is_ok());

        fs::set_permissions(daemon, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(DaemonPaths::from_home(Some(&home), false).is_err());
        assert!(DaemonPaths::from_home(Some(&home), true).is_err());
        assert_eq!(daemon.metadata().unwrap().mode() & 0o777, 0o755);
    }

    #[test]
    fn unsupported_socket_paths_fail_before_creating_daemon_directories() {
        let fixture = fs_fixture!({ ".keep": "" });
        let home = fixture.path("x".repeat(150));
        fs::create_dir(&home).unwrap();
        let error = DaemonPaths::from_home(Some(&home), true)
            .err()
            .expect("long socket path rejected");
        assert!(error.to_string().contains("unsupported daemon socket path"));
        assert!(!home.join(".turbotokens").exists());
    }

    #[test]
    fn pid_lock_rejects_links_and_serializes_daemons() {
        let fixture = fs_fixture!({ "victim": "keep me" });
        let pid = fixture.path("daemon.pid");
        let victim = fixture.path("victim");
        symlink(&victim, &pid).unwrap();
        assert!(PidFile::acquire(&pid).is_err());
        assert!(read_pid(&pid).is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
        fs::remove_file(&pid).unwrap();
        fs::set_permissions(&victim, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&victim, &pid).unwrap();
        assert!(PidFile::acquire(&pid).is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
        fs::remove_file(&pid).unwrap();

        let mut lock = PidFile::acquire(&pid).unwrap();
        lock.publish().unwrap();
        assert_eq!(read_pid(&pid).unwrap(), std::process::id());
        assert_eq!(pid.metadata().unwrap().mode() & 0o777, 0o600);
        assert!(PidFile::acquire(&pid).is_err());
        assert_eq!(read_pid(&pid).unwrap(), std::process::id());
        drop(lock);
        assert!(!pid.exists());
    }

    #[test]
    fn reuses_an_unlocked_stale_pid_without_signaling_it() {
        let fixture = fs_fixture!({ "daemon.pid": "999999" });
        let pid = fixture.path("daemon.pid");
        fs::set_permissions(&pid, fs::Permissions::from_mode(0o600)).unwrap();
        let mut lock = PidFile::acquire(&pid).unwrap();
        lock.publish().unwrap();
        assert_eq!(read_pid(&pid).unwrap(), std::process::id());
    }

    #[test]
    fn socket_bind_preserves_live_sockets_and_unrelated_files() {
        let fixture = fs_fixture!({ "victim": "keep me" });
        let socket = fixture.path("daemon.sock");
        symlink(fixture.path("victim"), &socket).unwrap();
        assert!(bind_socket(&socket).is_err());
        assert_eq!(
            fs::read_to_string(fixture.path("victim")).unwrap(),
            "keep me"
        );
        fs::remove_file(&socket).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let original = socket.metadata().unwrap();
        assert!(bind_socket(&socket).is_err());
        assert!(same_file(&original, &socket.metadata().unwrap()));
        drop(listener);
        let (_listener, guard) = bind_socket(&socket).unwrap();
        assert_eq!(socket.metadata().unwrap().mode() & 0o777, 0o600);
        drop(guard);
        assert!(!socket.exists());
    }
}
