//! Named keys. `keys/NAME.toml` holds public metadata only. The private key,
//! when this device has one, lives in `secrets/NAME` (mode 0600 in a 0700
//! directory), encoded in the selected rail's signer format.
use crate::{
    error::{Error, Result, quoted},
    routing::{self, Family},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Key {
    pub name: String,
    /// The canonical rail the key was created or added on.
    pub rail: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    pub curve: String,
    /// True when this device holds the private key.
    #[serde(default)]
    pub secret: bool,
}

impl Key {
    /// The public view that the keys commands print.
    pub(crate) fn view(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "rail": self.rail,
            "address": self.address,
            "publicKey": self.public_key,
            "curve": self.curve,
            "secret": self.secret,
        })
    }
    pub(crate) fn family(&self) -> Result<Family> {
        routing::family(&self.rail)
    }
    /// Refuses a key from another chain family.
    pub(crate) fn check_rail(&self, rail: &str) -> Result<()> {
        if self.family()? != routing::family(rail)? {
            return Err(Error::config(format!(
                "key {} belongs to {}, not {rail}",
                quoted(&self.name),
                self.rail
            )));
        }
        Ok(())
    }
}

pub(crate) fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || name.starts_with(['.', '-'])
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::usage(format!(
            "invalid key name {}: use 1 to 64 letters, digits, dots, underscores or hyphens",
            quoted(name)
        )));
    }
    Ok(())
}

fn metadata_path(dir: &Path, name: &str) -> PathBuf {
    dir.join("keys").join(format!("{name}.toml"))
}
pub(crate) fn secret_path(dir: &Path, name: &str) -> PathBuf {
    dir.join("secrets").join(name)
}

pub(crate) fn exists(dir: &Path, name: &str) -> bool {
    metadata_path(dir, name).exists() || secret_path(dir, name).exists()
}

pub(crate) fn load(dir: &Path, name: &str) -> Result<Key> {
    valid_name(name)?;
    let path = metadata_path(dir, name);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::config(format!("no key {}; run allowit keys ls", quoted(name)))
        } else {
            Error::config(format!("cannot read {}", path.display()))
        }
    })?;
    let key: Key = toml::from_str(&text)
        .map_err(|_| Error::config(format!("{} is invalid", path.display())))?;
    if key.name != name {
        return Err(Error::config(format!(
            "{} names another key",
            path.display()
        )));
    }
    Ok(key)
}

/// Every key, by name, and the metadata files that do not load.
pub(crate) fn list(dir: &Path) -> Result<(Vec<Key>, Vec<PathBuf>)> {
    let mut names = vec![];
    match std::fs::read_dir(dir.join("keys")) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let file = entry.file_name();
                if let Some(name) = file.to_str().and_then(|n| n.strip_suffix(".toml"))
                    && valid_name(name).is_ok()
                {
                    names.push(name.to_owned());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::config("cannot read the keys directory")),
    }
    names.sort();
    let (mut keys, mut invalid) = (vec![], vec![]);
    for name in names {
        match load(dir, &name) {
            Ok(key) => keys.push(key),
            Err(_) => invalid.push(metadata_path(dir, &name)),
        }
    }
    Ok((keys, invalid))
}

fn save(dir: &Path, key: &Key) -> Result<()> {
    let text = toml::to_string(key).map_err(|_| Error::config("cannot encode key metadata"))?;
    super::write_atomic(&metadata_path(dir, &key.name), text.as_bytes(), 0o644)
}

fn refuse_existing(dir: &Path, name: &str) -> Result<()> {
    valid_name(name)?;
    if exists(dir, name) {
        return Err(Error::config(format!(
            "key {} already exists; choose another name or run allowit keys rm {name}",
            quoted(name)
        )));
    }
    Ok(())
}

/// Generates a key with the rail's curve and stores its private key.
pub(crate) fn generate(dir: &Path, name: &str, rail: &str) -> Result<Key> {
    routing::info(rail)?;
    let _lock = super::lock(dir)?;
    refuse_existing(dir, name)?;
    super::private_dir(&dir.join("secrets"))?;
    let public = crate::wallet::generate_wallet(rail, &secret_path(dir, name))?;
    let key = Key {
        name: name.into(),
        rail: routing::normalize(rail)?.into(),
        address: public.address,
        public_key: Some(public.public_key),
        curve: public.key_type,
        secret: true,
    };
    save(dir, &key)?;
    Ok(key)
}

/// Records a public address that this device cannot sign for.
pub(crate) fn add(dir: &Path, name: &str, rail: &str, address: &str) -> Result<Key> {
    let _lock = super::lock(dir)?;
    refuse_existing(dir, name)?;
    let info = routing::info(rail)?;
    crate::vendors::address::check_account(info.family, address)?;
    let caps = crate::vendors::rail(info).caps();
    let key = Key {
        name: name.into(),
        rail: info.id.into(),
        address: address.into(),
        public_key: caps.address_is_key.then(|| address.to_owned()),
        curve: caps.curve.into(),
        secret: false,
    };
    save(dir, &key)?;
    Ok(key)
}

/// Stores an existing private key in the rail's signer format.
pub(crate) fn import_secret(dir: &Path, name: &str, rail: &str, secret: &[u8]) -> Result<Key> {
    routing::info(rail)?;
    let _lock = super::lock(dir)?;
    refuse_existing(dir, name)?;
    super::private_dir(&dir.join("secrets"))?;
    let path = secret_path(dir, name);
    write_private(&path, secret)?;
    let public = match crate::wallet::inspect_wallet(rail, &path) {
        Ok(public) => public,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return Err(Error::usage(format!(
                "the input is not a {} private key or a supported executor bundle",
                routing::family(rail)?.name()
            )));
        }
    };
    let key = Key {
        name: name.into(),
        rail: routing::normalize(rail)?.into(),
        address: public.address,
        public_key: Some(public.public_key),
        curve: public.key_type,
        secret: true,
    };
    save(dir, &key)?;
    Ok(key)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|_| Error::config("cannot create the private key file"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| Error::config("cannot save the private key"))
}

/// Reads the private key for `keys export`.
pub(crate) fn read_secret(dir: &Path, key: &Key) -> Result<zeroize::Zeroizing<String>> {
    if !key.secret {
        return Err(Error::config(format!(
            "key {} has no private key on this device",
            quoted(&key.name)
        )));
    }
    // Inspect first: it refuses symlinks, shared links and unsafe modes.
    crate::wallet::inspect_wallet(&key.rail, &secret_path(dir, &key.name))?;
    std::fs::read_to_string(secret_path(dir, &key.name))
        .map(zeroize::Zeroizing::new)
        .map_err(|_| Error::config("cannot read the private key"))
}

/// The private key file of a key that can sign.
pub(crate) fn signer_path(dir: &Path, key: &Key) -> Result<PathBuf> {
    if !key.secret {
        return Err(Error::config(format!(
            "key {} has no private key on this device; it cannot sign",
            quoted(&key.name)
        )));
    }
    Ok(secret_path(dir, &key.name))
}

pub(crate) fn remove(dir: &Path, name: &str) -> Result<()> {
    let _lock = super::lock(dir)?;
    let key = load(dir, name)?;
    if key.secret {
        match std::fs::remove_file(secret_path(dir, name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::config("cannot remove the private key")),
        }
    }
    std::fs::remove_file(metadata_path(dir, name))
        .map_err(|_| Error::config("cannot remove the key metadata"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn unknown_rail_writes_no_key() {
        let dir = crate::settings::temp_dir("unbuilt-keys");
        let secret = b"[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]";
        assert_eq!(
            generate(&dir, "a", "stellar:futurenet").unwrap_err().code,
            2
        );
        assert_eq!(
            import_secret(&dir, "b", "stellar:futurenet", secret)
                .unwrap_err()
                .code,
            2
        );
        assert!(!dir.join("secrets").exists() && !dir.join("keys").exists());
    }
    #[test]
    fn generate_add_import_and_remove_keep_secrets_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::settings::temp_dir("keys");
        let agent = generate(&dir, "agent", "stellar:testnet").unwrap();
        assert!(agent.address.starts_with('G'));
        assert_eq!(agent.public_key.as_deref(), Some(agent.address.as_str()));
        assert_eq!(agent.curve, "ed25519");
        let mode = std::fs::metadata(secret_path(&dir, "agent"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let mode = std::fs::metadata(dir.join("secrets"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        let metadata = std::fs::read_to_string(metadata_path(&dir, "agent")).unwrap();
        let secret = read_secret(&dir, &agent).unwrap();
        assert!(!metadata.contains(secret.trim()));
        assert_eq!(
            generate(&dir, "agent", "stellar:testnet").unwrap_err().code,
            3
        );

        let copy = import_secret(&dir, "copy", "stellar:pubnet", secret.as_bytes()).unwrap();
        assert_eq!(copy.address, agent.address);
        assert_eq!(copy.rail, "stellar:pubnet");
        assert!(import_secret(&dir, "bad", "stellar:testnet", b"not a key").is_err());
        assert!(!secret_path(&dir, "bad").exists());

        let owner = add(&dir, "owner", "stellar:testnet", &agent.address).unwrap();
        assert!(!owner.secret);
        assert!(signer_path(&dir, &owner).is_err());
        assert!(add(&dir, "other", "stellar:testnet", "GBAD").is_err());

        assert_eq!(
            list(&dir)
                .unwrap()
                .0
                .iter()
                .map(|k| k.name.as_str())
                .collect::<Vec<_>>(),
            ["agent", "copy", "owner"]
        );
        remove(&dir, "agent").unwrap();
        assert!(!secret_path(&dir, "agent").exists());
        assert!(load(&dir, "agent").is_err());
        assert!(valid_name("../x").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
