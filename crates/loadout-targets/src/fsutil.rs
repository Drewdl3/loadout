//! Filesystem helpers shared by Loadout's writers.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Suffix of the backup kept next to a file before Loadout overwrites it
///.
pub const BACKUP_SUFFIX: &str = ".loadout.bak";

/// The backup path for `path`: `<path>.loadout.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(BACKUP_SUFFIX);
    path.with_file_name(name)
}

/// Writes `contents` to `path` atomically: the data goes to a temporary file
/// in the same directory, is flushed to disk, and is then renamed over
/// `path`. Parent directories are created as needed.
///
/// With `backup`, an existing file at `path` is first copied to
/// [`backup_path`]; the backup holds the last pre-write version.
pub fn atomic_write(path: &Path, contents: &[u8], backup: bool) -> io::Result<()> {
    atomic_write_opts(
        path,
        contents,
        WriteOptions {
            backup,
            private: false,
        },
    )
}

/// Options for [`atomic_write_opts`].
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// Keep the previous version at [`backup_path`].
    pub backup: bool,
    /// Restrict the file to its owner (mode `0600` on Unix), e.g. because
    /// it holds secrets. Otherwise an existing file keeps its permissions.
    pub private: bool,
}

/// [`atomic_write`] with options.
pub fn atomic_write_opts(path: &Path, contents: &[u8], opts: WriteOptions) -> io::Result<()> {
    let backup = opts.backup;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".loadout-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(contents)?;
    if opts.private {
        set_private(tmp.as_file())?;
    } else if let Ok(meta) = std::fs::metadata(path) {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    tmp.as_file().sync_all()?;
    if backup && path.is_file() {
        std::fs::copy(path, backup_path(path))?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(unix)]
fn set_private(f: &std::fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))
}

/// Windows user-profile files are private to the user by default ACLs.
#[cfg(not(unix))]
fn set_private(_: &std::fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_new_file_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/config.toml");
        atomic_write(&path, b"x = 1\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x = 1\n");
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn keeps_backup_of_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "old").unwrap();
        atomic_write(&path, b"new", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(backup_path(&path)).unwrap(), "old");
        atomic_write(&path, b"newer", true).unwrap();
        assert_eq!(std::fs::read_to_string(backup_path(&path)).unwrap(), "new");
    }

    #[test]
    fn no_temp_files_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        atomic_write(&path, b"1", false).unwrap();
        atomic_write(&path, b"2", false).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("f")]);
    }

    #[cfg(unix)]
    #[test]
    fn keeps_or_restricts_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, "x").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write(&path, b"y", false).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o644);
        let opts = WriteOptions {
            backup: false,
            private: true,
        };
        atomic_write_opts(&path, b"z", opts).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn backup_path_appends_suffix() {
        assert_eq!(
            backup_path(Path::new("/h/.claude.json")),
            PathBuf::from("/h/.claude.json.loadout.bak")
        );
    }
}
