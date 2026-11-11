//! Terminal bindings for shared server-backed policy generation.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    kinds::{self, WalletKind},
    owner,
};
use allowit_sdk::generation::{self, GenerateRequest, Target};

pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy kinds\n  allowit policy generate --owner-key owner --origin http://localhost:5173 --kind standard --new-wallet 'Pay only my approved recipient'";

/// Generate a portable draft through the owner-authenticated server; does not deploy or spend
#[derive(clap::Args, Debug)]
#[command(group(clap::ArgGroup::new("generation_target").args(["new_wallet","installation"]).required(true)))]
pub(crate) struct Args {
    /// Complete owner instructions, preserved exactly
    pub prompt: String,
    /// Named local owner key; independent of the selected agent key
    #[arg(long, value_name = "NAME")]
    pub owner_key: String,
    /// Configured sign-in Origin, which can differ from ALLOWIT_URL
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
    // The command tree replaces this help with the SDK's kinds.
    /// Wallet kind
    #[arg(long, value_name = "KIND", value_parser = kinds::parse)]
    pub kind: WalletKind,
    /// Explicitly create a draft from an advertised wallet template
    #[arg(long)]
    pub new_wallet: bool,
    /// Existing owner installation ID (64 lowercase hexadecimal characters)
    #[arg(long, value_name = "ID")]
    pub installation: Option<String>,
}
pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    if cx.global.wallet_flag {
        return Err(Error::usage(
            "Generation selects --new-wallet or --installation; omit --wallet",
        ));
    }
    let rail = cx.rail()?;
    // The rail decides which kinds it supports and which profile installs each.
    kinds::require(rail.id, args.kind)?;
    let request = GenerateRequest {
        prompt: args.prompt,
        network: rail.id.into(),
        profile: kinds::profile(rail.id, args.kind)?.into(),
        target: if args.new_wallet {
            Target::NewWallet
        } else {
            Target::Installation(
                args.installation
                    .ok_or_else(|| Error::usage("Select --new-wallet or --installation"))?,
            )
        },
        // The server binds this kind or refuses it with a typed error.
        wallet_kind: Some(args.kind),
    };
    // Input checks come before any key or network access.
    request.validate()?;
    let server = cx.server_url()?;
    let signer = owner::signer(cx, &args.owner_key)?;
    crate::flush_stderr(&mut cx.out.stderr);
    let draft = generation::generate(&server, &args.origin, &signer, &request, owner::clock)?;
    check_bound(cx, args.kind, &draft.document)?;
    let code = if draft.unresolved.is_empty() { 0 } else { 11 };
    let review = Review::of(cx, rail.id, &draft.document);
    if cx.out.json {
        // stdout stays the draft document, which `policy deploy` reads. As a
        // progress step does, the review prints on stderr only with `-v`.
        if cx.out.verbose && !cx.out.quiet {
            cx.out.stderr.push_str(&format!("{}\n", review.json()));
        }
        cx.out.json_value(&draft.document);
    } else {
        cx.out.stdout.push_str(&review.text());
        cx.out.line("");
        cx.out.text(&draft.document);
    }
    Ok(code)
}

/// What an owner reviews before deployment: the wallet kind that the server
/// bound and each param in the unit of the installed asset, from the SDK.
struct Review {
    kind: Option<WalletKind>,
    params: Vec<allowit_sdk::kinds::ParamLine>,
}
impl Review {
    /// Reads the review of a draft. A draft whose params the rail cannot
    /// read keeps its document and gets a warning.
    fn of(cx: &mut Ctx, network: &str, draft: &serde_json::Value) -> Self {
        let params = allowit_sdk::kinds::param_lines(network, draft).unwrap_or_else(|error| {
            cx.out.warning(format!(
                "the params of the draft cannot be shown in asset units: {}",
                error.message
            ));
            vec![]
        });
        Self {
            kind: generation::wallet_kind_of(draft),
            params,
        }
    }
    /// `key  value` rows: the bound kind, then one `params.KEY` row per param.
    fn text(&self) -> String {
        let kind = self
            .kind
            .map(|kind| (kinds::KEY.to_owned(), kind.id().to_owned()));
        let params = self
            .params
            .iter()
            .map(|line| (format!("params.{}", line.key), line.value.clone()));
        crate::term::render(&kind.into_iter().chain(params).collect::<Vec<_>>())
    }
    /// One `{"review":…}` object for stderr in JSON mode.
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"review": {kinds::KEY: self.kind, "params": self.params}})
    }
}

/// Compares the kind that the server bound to the draft with the selection.
/// A draft of another kind is never printed as the result; it stays in the
/// owner workspace on the server, and the command exits 5, as the rail does
/// for the same reply. The bound kind is the draft's top-level `walletKind`;
/// a draft without it gets a warning, because the CLI cannot confirm the
/// selection.
fn check_bound(cx: &mut Ctx, selected: WalletKind, draft: &serde_json::Value) -> Result<()> {
    match kinds::bound(draft)? {
        Some(bound) if bound == selected => Ok(()),
        Some(bound) => Err(Error::uncertain(format!(
            "The server bound wallet kind {bound} to the draft, not the selected {selected}; the CLI did not print that draft"
        ))),
        None => {
            cx.out.warning(format!(
                "the draft does not report its wallet kind; confirm that it is {selected} before deployment"
            ));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::Global, term::Out};
    use serde_json::json;

    fn cx() -> Ctx {
        Ctx::new(Global::default(), Out::new(false, true, false, true))
    }

    #[test]
    fn the_bound_kind_must_be_the_selected_kind() {
        let mut cx = cx();
        let draft = json!({"id": "policy-0001", "walletKind": "standard"});
        check_bound(&mut cx, WalletKind::Standard, &draft).unwrap();
        assert!(cx.out.stderr.is_empty());
        let error = check_bound(&mut cx, WalletKind::Simple, &draft).unwrap_err();
        assert_eq!(error.code, 5);
        assert_eq!(
            error.message,
            "The server bound wallet kind standard to the draft, not the selected simple; the CLI did not print that draft"
        );
        assert_eq!(
            check_bound(
                &mut cx,
                WalletKind::Simple,
                &json!({"walletKind": "common"})
            )
            .unwrap_err()
            .code,
            5
        );
    }

    #[test]
    fn the_review_prints_the_bound_kind_and_each_param_in_asset_units() {
        allowit_sdk::plugins::testkit::each(
            allowit_sdk::plugins::Feature::DraftDisplay,
            |rail, fixtures| {
                let network = rail.test_support().network;
                // The fixture is a generation reply; its draft is under `policy`.
                let mut draft = fixtures
                    .owner_generation
                    .get("policy")
                    .unwrap_or(&fixtures.owner_generation)
                    .clone();
                let kind = rail.wallet_kinds(network)[0];
                draft[kinds::KEY] = kind.id().into();
                let lines = allowit_sdk::kinds::param_lines(network, &draft).unwrap();
                assert!(!lines.is_empty());
                let mut cx = cx();
                let review = Review::of(&mut cx, network, &draft);
                assert!(cx.out.stderr.is_empty(), "{}", cx.out.stderr);
                let text = review.text();
                let mut rows = text.lines();
                let first = rows.next().unwrap();
                assert!(first.starts_with("walletKind") && first.ends_with(kind.id()));
                for line in &lines {
                    let row = rows.next().unwrap();
                    assert!(row.starts_with(&format!("params.{} ", line.key)), "{row}");
                    assert!(row.ends_with(&line.value), "{row}");
                }
                assert!(!text.contains("UNIT"));
                // JSON mode keeps the same facts in one object.
                assert_eq!(
                    review.json(),
                    json!({"review": {"walletKind": kind.id(), "params": lines}})
                );
            },
        );
    }

    #[test]
    fn a_draft_that_the_rail_cannot_read_keeps_its_kind_and_warns() {
        for rail in crate::routing::all() {
            let mut cx = cx();
            let review = Review::of(&mut cx, rail.id, &json!({"walletKind": "custom"}));
            assert_eq!(review.text(), "walletKind  custom\n");
            assert_eq!(
                review.json(),
                json!({"review": {"walletKind": "custom", "params": []}})
            );
            assert!(
                cx.out.stderr.starts_with(
                    "warning: the params of the draft cannot be shown in asset units: "
                )
            );
        }
    }

    #[test]
    fn a_draft_without_a_kind_warns_and_keeps_the_result() {
        let mut cx = cx();
        check_bound(&mut cx, WalletKind::Simple, &json!({"id": "policy-0001"})).unwrap();
        assert_eq!(
            cx.out.stderr,
            "warning: the draft does not report its wallet kind; confirm that it is simple before deployment\n"
        );
    }
}
