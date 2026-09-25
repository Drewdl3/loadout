//! Where Loadout keeps its files.
//!
//! Defaults follow the `directories` crate conventions. Each root can be
//! overridden with an environment variable, which tests use to sandbox the
//! home directory on every OS:
//!
//! | Root | Default (Linux) | Override |
//! |---|---|---|
//! | config | `$XDG_CONFIG_HOME/loadout` | `LOADOUT_CONFIG_DIR` |
//! | data | `$XDG_DATA_HOME/loadout` | `LOADOUT_DATA_DIR` |
//! | home (for `~` in target paths) | `$HOME` | `LOADOUT_HOME` |

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};

const APP: &str = "loadout";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    /// Project mode: the repo whose `.loadout/` this is.
    pub project: Option<Project>,
}

/// A project layer: config and lock live in the repo (committed); local
/// state lives in the user's data dir; clones are shared with the user
/// layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    /// The user layer's `repos/` (clones are shared).
    pub repos: PathBuf,
}

/// Directory holding a project's Loadout files.
pub const PROJECT_DIR: &str = ".loadout";

/// The platform defaults, before overrides.
#[derive(Debug, Clone)]
pub struct Defaults {
    pub home: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Defaults {
    fn from_platform() -> Option<Self> {
        let base = directories::BaseDirs::new()?;
        Some(Defaults {
            home: base.home_dir().to_path_buf(),
            config_dir: base.config_dir().join(APP),
            data_dir: base.data_dir().join(APP),
        })
    }
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        Self::resolve(|k| std::env::var_os(k), Defaults::from_platform)
    }

    /// Applies environment overrides on top of lazily computed defaults.
    pub fn resolve(
        env: impl Fn(&str) -> Option<OsString>,
        defaults: impl FnOnce() -> Option<Defaults>,
    ) -> Result<Self> {
        let get = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let (home, config_dir, data_dir) = (
            get("LOADOUT_HOME"),
            get("LOADOUT_CONFIG_DIR"),
            get("LOADOUT_DATA_DIR"),
        );
        if let (Some(home), Some(config_dir), Some(data_dir)) = (&home, &config_dir, &data_dir) {
            return Ok(Paths {
                home: home.clone(),
                config_dir: config_dir.clone(),
                data_dir: data_dir.clone(),
                project: None,
            });
        }
        let d = defaults().context(
            "cannot determine the home directory; set LOADOUT_HOME, \
             LOADOUT_CONFIG_DIR and LOADOUT_DATA_DIR",
        )?;
        Ok(Paths {
            home: home.unwrap_or(d.home),
            config_dir: config_dir.unwrap_or(d.config_dir),
            data_dir: data_dir.unwrap_or(d.data_dir),
            project: None,
        })
    }

    /// The project layer of the repo at `root`.
    pub fn for_project(&self, root: &std::path::Path) -> Paths {
        let key = blake3::hash(root.to_string_lossy().as_bytes()).to_hex();
        Paths {
            home: self.home.clone(),
            config_dir: root.join(PROJECT_DIR),
            data_dir: self.data_dir.join("projects").join(&key[..16]),
            project: Some(Project {
                root: root.to_path_buf(),
                repos: self.repos_dir(),
            }),
        }
    }

    /// The project containing `dir`: the nearest ancestor with
    /// `.loadout/config.toml`.
    pub fn find_project(dir: &std::path::Path) -> Option<PathBuf> {
        dir.ancestors()
            .find(|d| d.join(PROJECT_DIR).join("config.toml").is_file())
            .map(std::path::Path::to_path_buf)
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// User target-table overrides (`targets/*.toml`).
    pub fn user_targets_dir(&self) -> PathBuf {
        self.config_dir.join("targets")
    }

    pub fn repos_dir(&self) -> PathBuf {
        match &self.project {
            Some(p) => p.repos.clone(),
            None => self.data_dir.join("repos"),
        }
    }

    pub fn store_dir(&self) -> PathBuf {
        self.data_dir.join("store")
    }

    pub fn owned_file(&self) -> PathBuf {
        self.data_dir.join("owned.json")
    }

    /// `loadout.lock`.
    pub fn lock_file(&self) -> PathBuf {
        match &self.project {
            // Committed with the project.
            Some(_) => self.config_dir.join("loadout.lock"),
            None => self.data_dir.join("loadout.lock"),
        }
    }

    /// `state.json`: last sync time and changes held for review.
    pub fn state_file(&self) -> PathBuf {
        self.data_dir.join("state.json")
    }

    /// The cached search index.
    pub fn search_file(&self) -> PathBuf {
        self.data_dir.join("search.json")
    }

    pub fn resolved_file(&self) -> PathBuf {
        self.data_dir.join("resolved.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn defaults() -> Option<Defaults> {
        Some(Defaults {
            home: "/h".into(),
            config_dir: "/h/.config/loadout".into(),
            data_dir: "/h/.local/share/loadout".into(),
        })
    }

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn uses_platform_defaults() {
        let p = Paths::resolve(env(&[]), defaults).unwrap();
        assert_eq!(
            p.config_file(),
            PathBuf::from("/h/.config/loadout/config.toml")
        );
        assert_eq!(
            p.store_dir(),
            PathBuf::from("/h/.local/share/loadout/store")
        );
    }

    #[test]
    fn env_overrides_win_and_empty_is_ignored() {
        let p = Paths::resolve(
            env(&[("LOADOUT_DATA_DIR", "/d"), ("LOADOUT_HOME", "")]),
            defaults,
        )
        .unwrap();
        assert_eq!(p.data_dir, PathBuf::from("/d"));
        assert_eq!(p.home, PathBuf::from("/h"));
    }

    #[test]
    fn full_override_needs_no_platform_dirs() {
        let p = Paths::resolve(
            env(&[
                ("LOADOUT_HOME", "/x"),
                ("LOADOUT_CONFIG_DIR", "/c"),
                ("LOADOUT_DATA_DIR", "/d"),
            ]),
            || None,
        )
        .unwrap();
        assert_eq!(p.home, PathBuf::from("/x"));
    }

    #[test]
    fn missing_home_is_an_error() {
        assert!(Paths::resolve(env(&[]), || None).is_err());
    }
}
