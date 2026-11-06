//! Local settings under the config directory (`$XDG_CONFIG_HOME/allowit`,
//! default `~/.config/allowit`, or `--config-dir`):
//!
//! - `config.toml`: the current rail and, per rail, the default wallet, the
//!   agent key, known wallets and an RPC override.
//! - `keys/NAME.toml`: public key metadata. `secrets/NAME`: the private key,
//!   mode 0600 in a 0700 directory.
//! - `requests/`: the local request journal that `status` and `history` read.
//! - `.lock`: held while a command edits `config.toml` or the key store.
pub(crate) mod journal;
pub(crate) mod keys;

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Settings for one rail, keyed by its canonical CAIP-2 identifier.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RailSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wallets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub server_authorities: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rail: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rails: BTreeMap<String, RailSettings>,
}

/// The config directory and its parsed `config.toml`.
pub(crate) struct Settings {
    pub dir: PathBuf,
    pub config: Config,
}

/// The default config directory, following XDG.
pub(crate) fn default_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("allowit"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join("allowit"))
        .ok_or_else(|| Error::config("HOME is not set; pass --config-dir DIR"))
}

impl Settings {
    pub(crate) fn path(dir: &Path) -> PathBuf {
        dir.join("config.toml")
    }
    /// Reads `config.toml`. A missing file is an empty configuration.
    pub(crate) fn load(dir: PathBuf) -> Result<Self> {
        let path = Self::path(&dir);
        let config = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| {
                Error::config(format!("{} is invalid: {}", path.display(), e.message()))
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(_) => return Err(Error::config(format!("cannot read {}", path.display()))),
        };
        Ok(Self { dir, config })
    }
    /// Applies `change` to the current `config.toml` under the directory
    /// lock and writes the result, so concurrent commands keep each other's
    /// edits. `change` sees the file as it is now, not the cached snapshot.
    pub(crate) fn update<T>(&mut self, change: impl FnOnce(&mut Config) -> Result<T>) -> Result<T> {
        let _lock = lock(&self.dir)?;
        let mut current = Self::load(self.dir.clone())?;
        let before = current.config.clone();
        let value = change(&mut current.config)?;
        if current.config != before {
            current.save()?;
        }
        self.config = current.config;
        Ok(value)
    }
    /// Writes `config.toml` atomically.
    fn save(&self) -> Result<()> {
        let text = toml::to_string(&self.config)
            .map_err(|_| Error::config("cannot encode config.toml"))?;
        write_atomic(&Self::path(&self.dir), text.as_bytes(), 0o644)
    }
    pub(crate) fn rail(&self, id: &str) -> RailSettings {
        self.config.rails.get(id).cloned().unwrap_or_default()
    }
}

impl Config {
    pub(crate) fn rail_mut(&mut self, id: &str) -> &mut RailSettings {
        self.rails.entry(id.to_owned()).or_default()
    }
}

/// The exclusive lock on a config directory, released on drop.
pub(crate) struct Lock {
    _file: std::fs::File,
}

/// Waits for the exclusive lock on `dir/.lock`. Every read-modify-write of
/// `config.toml` or the key store holds it.
pub(crate) fn lock(dir: &Path) -> Result<Lock> {
    let path = dir.join(".lock");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| Error::config(format!("cannot open {}", path.display())))?;
    file.lock()
        .map_err(|_| Error::config(format!("cannot lock {}", path.display())))?;
    Ok(Lock { _file: file })
}

/// Creates a private directory (mode 0700) and its parents.
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| Error::config(format!("cannot create {}", path.display())))
}

/// Writes a file through a temporary file and a rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| Error::config("invalid settings path"))?;
    private_dir(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| Error::config(format!("cannot write {}", path.display())))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| Error::config(format!("cannot write {}", path.display())))?;
        std::fs::rename(&temporary, path)
            .map_err(|_| Error::config(format!("cannot write {}", path.display())))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "allowit-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    private_dir(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_round_trips_and_missing_file_is_empty() {
        let dir = temp_dir("settings");
        let mut s = Settings::load(dir.clone()).unwrap();
        assert_eq!(s.config, Config::default());
        s.update(|c| {
            c.rail = Some("stellar:testnet".into());
            c.rail_mut("stellar:testnet").wallet = Some("CWALLET".into());
            Ok(())
        })
        .unwrap();
        let again = Settings::load(dir.clone()).unwrap();
        assert_eq!(again.config, s.config);
        let text = std::fs::read_to_string(Settings::path(&dir)).unwrap();
        assert!(text.contains("[rails.\"stellar:testnet\"]"), "{text}");
        std::fs::write(Settings::path(&dir), "rail = 3").unwrap();
        assert_eq!(Settings::load(dir.clone()).err().unwrap().code, 3);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
