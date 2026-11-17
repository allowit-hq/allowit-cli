//! Wallet kinds as the SDK describes them. The four kinds, their text and the
//! kinds that a rail supports all come from the SDK; this file only lays
//! them out for the terminal.
use crate::error::{Error, Result, quoted};
use allowit_sdk::kinds::{Enforcement, WalletCustody};
pub(crate) use allowit_sdk::kinds::{WalletKind, WalletKindInfo};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The key of the bound kind in every draft, policy and wallet document.
pub(crate) const KEY: &str = "walletKind";

/// The selection ids, in selection order: `native, simple, standard, custom`.
fn ids() -> String {
    WalletKind::ALL.map(WalletKind::id).join(", ")
}

/// The `--kind` value parser. Ids are exact; there are no aliases.
pub(crate) fn parse(text: &str) -> std::result::Result<WalletKind, String> {
    WalletKind::parse(text)
        .ok_or_else(|| format!("use one of {}; allowit policy kinds explains each", ids()))
}

/// The one-line help of `--kind`.
pub(crate) fn flag_help() -> String {
    format!(
        "Wallet kind: {}; allowit policy kinds lists the selected rail's kinds",
        ids()
    )
}

/// The help section that explains every kind: its title and explanation, the
/// sellers that it pays, its network fee and, when the kind has a fixed list
/// of params, where that list is. The recommended kind is marked.
pub(crate) fn help_section() -> String {
    let width = WalletKind::ALL
        .iter()
        .map(|kind| kind.id().len())
        .max()
        .unwrap_or(0);
    let mut text = String::from("Wallet kinds:");
    for kind in WalletKind::ALL {
        let info = kind.info();
        text.push_str(&format!(
            "\n  {:width$}  {}{}: {}",
            kind.id(),
            info.title,
            if info.recommended {
                " (recommended)"
            } else {
                ""
            },
            info.explanation
        ));
        text.push_str(&format!("\n  {:width$}  Pays: {}", "", info.pays));
        text.push_str(&format!("\n  {:width$}  Network fee: {}", "", info.fee));
        if let Some(url) = info.params_url {
            text.push_str(&format!("\n  {:width$}  Supported params: {url}", ""));
        }
    }
    text
}

/// The kinds that a rail supports, in selection order.
pub(crate) fn supported(network: &str) -> Result<Vec<&'static WalletKindInfo>> {
    allowit_sdk::kinds::wallet_kinds(network)
}

/// Refuses a kind that the rail does not support, before any key or network access.
pub(crate) fn require(network: &str, kind: WalletKind) -> Result<()> {
    let kinds = supported(network)?;
    if kinds.iter().any(|info| info.id == kind) {
        return Ok(());
    }
    let offered: Vec<_> = kinds.iter().map(|info| info.id.id()).collect();
    Err(Error::unsupported(format!(
        "wallet kind {} is not supported on {network}; it supports {}",
        kind.id(),
        if offered.is_empty() {
            "none".to_owned()
        } else {
            offered.join(", ")
        }
    )))
}

/// The rail's wallet profile that installs `kind`. A rail that installs
/// Simple and Standard through one wallet contract advertises one profile for
/// both. The SDK request names the profile, so the draft's `walletKind` is
/// what confirms the kind that the server bound.
pub(crate) fn profile(network: &str, kind: WalletKind) -> Result<&'static str> {
    let profiles = allowit_sdk::plugins::for_network(network)?
        .server()
        .profiles();
    let of = |wanted: WalletKind| {
        profiles
            .iter()
            .find(|profile| profile.wallet_kind() == Some(wanted))
            .map(|profile| profile.id)
    };
    let shared = match kind {
        WalletKind::Simple => Some(WalletKind::Standard),
        WalletKind::Standard => Some(WalletKind::Simple),
        WalletKind::Native | WalletKind::Custom => None,
    };
    of(kind).or_else(|| shared.and_then(of)).ok_or_else(|| {
        Error::unsupported(format!(
            "{network} advertises no wallet profile for kind {}",
            kind.id()
        ))
    })
}

/// The kind that a server document binds, from its top-level `walletKind`.
/// `None` when the document does not carry the field.
pub(crate) fn bound(document: &Value) -> Result<Option<WalletKind>> {
    match document.get(KEY) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(WalletKind::parse)
            .map(Some)
            .ok_or_else(|| {
                Error::uncertain(format!(
                    "The server reported an unknown wallet kind {}",
                    quoted(&value.to_string())
                ))
            }),
    }
}

/// One kind as the terminal prints it, with the JSON fields of the SDK's
/// description. The SDK supplies it for a rail, or the server for itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Kind {
    pub id: WalletKind,
    pub title: String,
    pub explanation: String,
    /// The kind that hosts recommend to an owner who deploys.
    #[serde(default)]
    pub recommended: bool,
    /// The sellers that a wallet of this kind pays. Empty from a server that does not say.
    #[serde(default)]
    pub pays: String,
    /// What a payment costs the wallet in network fee. Empty from a server that does not say.
    #[serde(default)]
    pub fee: String,
    pub enforcement: Enforcement,
    pub custody: WalletCustody,
    #[serde(default)]
    pub supported_params: Vec<String>,
    #[serde(default)]
    pub params_url: Option<String>,
}
impl From<&WalletKindInfo> for Kind {
    fn from(info: &WalletKindInfo) -> Self {
        Self {
            id: info.id,
            title: info.title.into(),
            explanation: info.explanation.into(),
            recommended: info.recommended,
            pays: info.pays.into(),
            fee: info.fee.into(),
            enforcement: info.enforcement,
            custody: info.custody,
            supported_params: info.supported_params.iter().map(|p| (*p).into()).collect(),
            params_url: info.params_url.map(String::from),
        }
    }
}
impl Kind {
    /// The text rows of the kind, named as its JSON fields. A kind without
    /// a fixed list of params has no params rows.
    pub(crate) fn rows(&self) -> Vec<(&'static str, String)> {
        let word = |value: Value| value.as_str().unwrap_or_default().to_owned();
        let mut rows = vec![
            ("id", self.id.id().to_owned()),
            ("title", self.title.clone()),
            ("explanation", self.explanation.clone()),
        ];
        // The recommendation and what the owner compares: the sellers and the fee.
        if self.recommended {
            rows.push(("recommended", "yes".to_owned()));
        }
        if !self.pays.is_empty() {
            rows.push(("pays", self.pays.clone()));
        }
        if !self.fee.is_empty() {
            rows.push(("fee", self.fee.clone()));
        }
        rows.extend([
            (
                "enforcement",
                word(serde_json::to_value(self.enforcement).unwrap_or_default()),
            ),
            (
                "custody",
                word(serde_json::to_value(self.custody).unwrap_or_default()),
            ),
        ]);
        if !self.supported_params.is_empty() {
            rows.push(("supportedParams", self.supported_params.join(", ")));
        }
        if let Some(url) = &self.params_url {
            rows.push(("paramsUrl", url.clone()));
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_exact_and_the_retired_value_is_refused() {
        for kind in WalletKind::ALL {
            assert_eq!(parse(kind.id()), Ok(kind));
        }
        for bad in ["common", "Standard", "", "native "] {
            let message = parse(bad).unwrap_err();
            assert!(
                message.contains("native, simple, standard, custom"),
                "{message}"
            );
        }
    }

    #[test]
    fn help_takes_every_kind_and_the_params_link_from_the_sdk() {
        let help = help_section();
        for kind in WalletKind::ALL {
            let info = kind.info();
            let mark = if info.recommended {
                " (recommended)"
            } else {
                ""
            };
            assert!(help.contains(&format!("{}{mark}: {}", info.title, info.explanation)));
            assert!(help.contains(&format!("Pays: {}", info.pays)));
            assert!(help.contains(&format!("Network fee: {}", info.fee)));
            if let Some(url) = info.params_url {
                assert!(help.contains(url));
            }
        }
        // One kind is recommended, and help marks only that one.
        assert_eq!(help.matches("(recommended)").count(), 1);
        assert!(help.contains("Native (recommended): "));
        assert!(flag_help().contains("native, simple, standard, custom"));
    }

    #[test]
    fn a_listed_kind_shows_its_recommendation_its_sellers_and_its_fee() {
        let native = Kind::from(WalletKind::Native.info());
        let rows = native.rows();
        let row = |name: &str| {
            rows.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(row("recommended"), Some("yes"));
        assert_eq!(row("pays"), Some(WalletKind::Native.info().pays));
        assert_eq!(row("fee"), Some(WalletKind::Native.info().fee));
        let json = serde_json::to_value(&native).unwrap();
        assert_eq!(json["recommended"], true);
        assert_eq!(json["pays"], WalletKind::Native.info().pays);
        // Another kind has no recommendation row, and it states its sellers and its fee.
        let standard = Kind::from(WalletKind::Standard.info());
        assert!(standard.rows().iter().all(|(key, _)| *key != "recommended"));
        assert!(standard.rows().iter().any(|(key, _)| *key == "pays"));
        // A server of an earlier release lists a kind without these fields.
        let earlier: Kind = serde_json::from_value(serde_json::json!({
            "id": "native", "title": "Native", "explanation": "An earlier text.",
            "enforcement": "offchain", "custody": "keypair"
        }))
        .unwrap();
        assert!(!earlier.recommended && earlier.pays.is_empty() && earlier.fee.is_empty());
        assert!(
            earlier
                .rows()
                .iter()
                .all(|(key, _)| !["recommended", "pays", "fee"].contains(key))
        );
    }

    #[test]
    fn every_rail_resolves_a_profile_for_each_kind_it_supports() {
        for rail in crate::routing::all() {
            let kinds = supported(rail.id).unwrap();
            for info in &kinds {
                require(rail.id, info.id).unwrap();
                assert!(!profile(rail.id, info.id).unwrap().is_empty());
            }
            for kind in WalletKind::ALL {
                if !kinds.iter().any(|info| info.id == kind) {
                    assert_eq!(require(rail.id, kind).unwrap_err().code, 3);
                }
            }
        }
    }

    #[test]
    fn the_bound_kind_is_the_top_level_field() {
        use serde_json::json;
        assert_eq!(bound(&json!({"id": "p"})).unwrap(), None);
        assert_eq!(
            bound(&json!({"walletKind": "simple"})).unwrap(),
            Some(WalletKind::Simple)
        );
        for bad in [json!({"walletKind": "common"}), json!({"walletKind": 1})] {
            assert_eq!(bound(&bad).unwrap_err().code, 5);
        }
    }

    #[test]
    fn a_kind_keeps_the_json_of_the_sdk_and_its_rows_follow_the_fields() {
        for kind in WalletKind::ALL {
            let info = kind.info();
            let owned = Kind::from(info);
            let json = serde_json::to_value(info).unwrap();
            assert_eq!(serde_json::to_value(&owned).unwrap(), json);
            assert_eq!(serde_json::from_value::<Kind>(json.clone()).unwrap(), owned);
            for (key, text) in owned.rows() {
                match &json[key] {
                    Value::String(value) => assert_eq!(value, &text),
                    Value::Array(values) => assert_eq!(values.len(), text.split(", ").count()),
                    // Only the recommended kind has the row, and it says yes.
                    Value::Bool(true) => assert_eq!(text, "yes"),
                    other => panic!("{key}: {other}"),
                }
            }
        }
    }
}
