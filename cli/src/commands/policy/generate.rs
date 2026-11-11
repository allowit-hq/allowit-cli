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
    };
    // Input checks come before any key or network access.
    request.validate()?;
    let server = cx.server_url()?;
    let signer = owner::signer(cx, &args.owner_key)?;
    crate::flush_stderr(&mut cx.out.stderr);
    let draft = generation::generate(&server, &args.origin, &signer, &request, owner::clock)?;
    check_bound(cx, args.kind, &draft.document)?;
    let code = if draft.unresolved.is_empty() { 0 } else { 11 };
    cx.out.value(&draft.document);
    Ok(code)
}

/// Compares the kind that the server bound to the draft with the selection.
/// A draft of another kind is never printed as the result; it stays in the
/// owner workspace on the server. The bound kind is
/// the draft's top-level `walletKind`; a draft without it gets a warning,
/// because the CLI cannot confirm the selection.
fn check_bound(cx: &mut Ctx, selected: WalletKind, draft: &serde_json::Value) -> Result<()> {
    match kinds::bound(draft)? {
        Some(bound) if bound == selected => Ok(()),
        Some(bound) => Err(Error::unsupported(format!(
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
        assert_eq!(error.code, 3);
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
    fn a_draft_without_a_kind_warns_and_keeps_the_result() {
        let mut cx = cx();
        check_bound(&mut cx, WalletKind::Simple, &json!({"id": "policy-0001"})).unwrap();
        assert_eq!(
            cx.out.stderr,
            "warning: the draft does not report its wallet kind; confirm that it is simple before deployment\n"
        );
    }
}
