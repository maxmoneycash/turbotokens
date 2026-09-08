//! Shared location and directory creation rules for the Claude parse cache.

use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Eq, PartialEq)]
pub enum CacheRoot {
    Disabled,
    Unavailable,
    Dir(PathBuf),
}

pub fn cache_root_from_env() -> CacheRoot {
    cache_root_from_values(
        env::var_os("TURBOTOKENS_CACHE"),
        env::var_os("TURBOTOKENS_CACHE_DIR"),
        crate::home::home_dir(),
        env::var_os("XDG_CACHE_HOME"),
        env::var_os("LOCALAPPDATA"),
    )
}

fn cache_root_from_values(
    enabled: Option<OsString>,
    explicit: Option<OsString>,
    home: Option<PathBuf>,
    xdg: Option<OsString>,
    local_app_data: Option<OsString>,
) -> CacheRoot {
    if let Some(value) = enabled.as_deref().and_then(|value| value.to_str()) {
        let value = value.trim();
        if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("false") || value == "0"
        {
            return CacheRoot::Disabled;
        }
    }
    // An explicit override is intentionally trusted, including relative paths.
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        return CacheRoot::Dir(PathBuf::from(path));
    }
    let Some(home) = home.filter(|path| path.is_absolute() && path.is_dir()) else {
        return CacheRoot::Unavailable;
    };
    let base = if cfg!(target_os = "windows") {
        absolute_path(local_app_data).unwrap_or_else(|| home.join("AppData").join("Local"))
    } else if cfg!(target_os = "macos") {
        home.join("Library").join("Caches")
    } else {
        absolute_path(xdg).unwrap_or_else(|| home.join(".cache"))
    };
    CacheRoot::Dir(base.join("turbotokens"))
}

fn absolute_path(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Create cache directories privately on Unix. Existing directories, including
/// explicitly configured shared locations, keep their current permissions.
pub fn create_cache_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

#[cfg(test)]
mod tests {
    use turbotokens_test_support::fs_fixture;

    use super::*;

    #[test]
    fn disabling_cache_takes_precedence_over_an_explicit_directory() {
        for value in ["off", " FALSE ", "0"] {
            assert_eq!(
                cache_root_from_values(
                    Some(value.into()),
                    Some("trusted-cache".into()),
                    None,
                    None,
                    None,
                ),
                CacheRoot::Disabled,
            );
        }
    }

    #[test]
    fn explicit_directory_works_without_a_home() {
        assert_eq!(
            cache_root_from_values(None, Some("trusted-cache".into()), None, None, None),
            CacheRoot::Dir(PathBuf::from("trusted-cache")),
        );
    }

    #[test]
    fn default_cache_requires_an_existing_absolute_home() {
        let fixture = fs_fixture!({ "home/.keep": "" });
        for home in [
            None,
            Some(PathBuf::from("relative")),
            Some(fixture.path("missing")),
        ] {
            assert_eq!(
                cache_root_from_values(None, None, home, None, None),
                CacheRoot::Unavailable,
            );
        }
    }

    #[test]
    fn default_cache_uses_the_platform_cache_directory() {
        let fixture = fs_fixture!({ "home/.keep": "" });
        let home = fixture.path("home");
        let expected = if cfg!(target_os = "windows") {
            home.join("AppData").join("Local")
        } else if cfg!(target_os = "macos") {
            home.join("Library").join("Caches")
        } else {
            home.join(".cache")
        };

        assert_eq!(
            cache_root_from_values(None, Some("".into()), Some(home), None, None),
            CacheRoot::Dir(expected.join("turbotokens")),
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn honors_absolute_xdg_cache_home_and_ignores_relative_values() {
        let fixture = fs_fixture!({ "home/.keep": "" });
        let home = fixture.path("home");
        let xdg = fixture.path("xdg");
        assert_eq!(
            cache_root_from_values(
                None,
                None,
                Some(home.clone()),
                Some(xdg.clone().into()),
                None
            ),
            CacheRoot::Dir(xdg.join("turbotokens")),
        );
        assert_eq!(
            cache_root_from_values(
                None,
                None,
                Some(home.clone()),
                Some("relative".into()),
                None
            ),
            CacheRoot::Dir(home.join(".cache").join("turbotokens")),
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn honors_local_app_data() {
        let fixture = fs_fixture!({ "home/.keep": "" });
        let local = fixture.path("local");
        assert_eq!(
            cache_root_from_values(
                None,
                None,
                Some(fixture.path("home")),
                None,
                Some(local.clone().into())
            ),
            CacheRoot::Dir(local.join("turbotokens")),
        );
    }

    #[cfg(unix)]
    #[test]
    fn creates_private_cache_directories() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = fs_fixture!({ "home/.keep": "" });
        let root = fixture.path("home/cache/turbotokens");
        create_cache_dir(&root.join("parse-v1/claude")).unwrap();

        assert_eq!(
            fs::metadata(root).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
