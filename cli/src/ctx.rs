//! The command context: global flags, output and lazily loaded settings.
//! Precedence for every setting is flag, then environment, then the
//! configuration that the `use` commands write.
use crate::{
    commands::Global,
    error::{Error, Result, quoted},
    exit,
    routing::{self, RailInfo},
    settings::{self, Settings, keys},
    term::Out,
    vendors::{self, Rail},
};
use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
};

pub(crate) struct Ctx {
    pub global: Global,
    pub out: Out,
    settings: Option<Settings>,
}

impl Ctx {
    pub(crate) fn new(global: Global, out: Out) -> Self {
        Self {
            global,
            out,
            settings: None,
        }
    }
    /// The config directory, created on first use.
    pub(crate) fn dir(&mut self) -> Result<PathBuf> {
        Ok(self.settings()?.dir.clone())
    }
    pub(crate) fn settings(&mut self) -> Result<&mut Settings> {
        if self.settings.is_none() {
            let dir = match &self.global.config_dir {
                Some(dir) => dir.clone(),
                None => settings::default_dir()?,
            };
            settings::private_dir(&dir)?;
            // The private stores refuse symlinked path components.
            let dir = dir
                .canonicalize()
                .map_err(|_| Error::config(format!("cannot open {}", dir.display())))?;
            self.settings = Some(Settings::load(dir)?);
        }
        Ok(self.settings.as_mut().expect("settings loaded"))
    }
    /// The config directory without creating it, for `config path`.
    pub(crate) fn dir_path(&self) -> Result<PathBuf> {
        match &self.global.config_dir {
            Some(dir) => Ok(dir.clone()),
            None => settings::default_dir(),
        }
    }
    /// The selected rail, if any: `--rail`, `ALLOWIT_RAIL`, then `rail use`.
    pub(crate) fn rail_opt(&mut self) -> Result<Option<&'static RailInfo>> {
        if let Some(value) = self.global.rail.clone() {
            return routing::info(&value).map(Some);
        }
        match self.settings()?.config.rail.clone() {
            Some(value) => routing::info(&value).map(Some).map_err(|_| {
                Error::config(format!(
                    "config.toml selects unknown rail {}; run allowit rail use RAIL",
                    quoted(&value)
                ))
            }),
            None => Ok(None),
        }
    }
    /// The selected rail. Exits 3 with a hint when none is selected.
    pub(crate) fn rail(&mut self) -> Result<&'static RailInfo> {
        self.rail_opt()?.ok_or_else(exit::no_rail)
    }
    /// The vendor adapter of the selected rail.
    pub(crate) fn vendor(&mut self) -> Result<Box<dyn Rail>> {
        Ok(vendors::rail(self.rail()?))
    }
    /// The wallet: `--wallet`, `ALLOWIT_WALLET`, then `wallet use`. The
    /// address must match the rail.
    pub(crate) fn wallet(&mut self, rail: &RailInfo) -> Result<Option<String>> {
        let wallet = match self.global.wallet.clone() {
            Some(w) => Some(w),
            None => self.settings()?.rail(rail.id).wallet,
        };
        if let Some(w) = &wallet {
            vendors::address::check_wallet(rail.family, w)?;
        }
        Ok(wallet)
    }
    /// The local signing key that `keys use` chose for the rail.
    pub(crate) fn agent(&mut self, rail: &RailInfo) -> Result<keys::Key> {
        let name = self.settings()?.rail(rail.id).agent.ok_or_else(|| {
            Error::config(format!(
                "no agent key for {}; run allowit keys use NAME",
                rail.id
            ))
        })?;
        let key = self.key(&name)?;
        key.check_rail(rail.id)?;
        Ok(key)
    }
    pub(crate) fn key(&mut self, name: &str) -> Result<keys::Key> {
        let dir = self.dir()?;
        keys::load(&dir, name)
    }
    /// The private key file of a key.
    pub(crate) fn signer(&mut self, key: &keys::Key) -> Result<PathBuf> {
        let dir = self.dir()?;
        keys::signer_path(&dir, key)
    }
    /// The RPC override: `--rpc-url`, then the rail's `rpc_url` setting.
    pub(crate) fn rpc_url(&mut self, rail: &RailInfo) -> Result<Option<String>> {
        if let Some(url) = &self.global.rpc_url {
            return Ok(Some(url.clone()));
        }
        Ok(self.settings()?.rail(rail.id).rpc_url)
    }
    pub(crate) fn server_authority(
        &mut self,
        rail: &RailInfo,
    ) -> Result<allowit_sdk_engine::config::ServerAuthority> {
        let origin = crate::config::parse_origin(&crate::config::env_value("ALLOWIT_URL"))?;
        let key=self.settings()?.rail(rail.id).server_authorities.get(&origin).cloned().ok_or_else(||Error::config("Configure independent server authority with allowit rail trust ORIGIN PUBLIC_KEY"))?;
        allowit_sdk_engine::config::ServerAuthority::new(rail.id, &origin, &key)
    }
    /// Confirms a destructive action: `--yes`, or a prompt on a terminal.
    pub(crate) fn confirm(&mut self, question: &str) -> Result<()> {
        if self.global.yes {
            return Ok(());
        }
        if self.out.compact || !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return Err(Error::usage(format!("{question}: pass --yes to confirm")));
        }
        crate::flush_stderr(&mut self.out.stderr);
        eprint!("{question} [y/N] ");
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|_| Error::usage("cannot read the confirmation"))?;
        if matches!(answer.trim(), "y" | "Y" | "yes") {
            Ok(())
        } else {
            Err(Error::usage("cancelled"))
        }
    }
}

/// Reads a file argument, or stdin for `-`.
pub(crate) fn read_input(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = vec![];
    let limit = 2 << 20;
    if path == Path::new("-") {
        std::io::stdin()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::usage("cannot read stdin"))?;
    } else {
        std::fs::File::open(path)
            .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
            .map_err(|_| Error::usage(format!("cannot read {}", path.display())))?;
    }
    if bytes.len() as u64 > limit {
        return Err(Error::usage("the input exceeds 2 MiB"));
    }
    Ok(bytes)
}
