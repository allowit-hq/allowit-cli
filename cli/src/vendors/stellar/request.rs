//! CLI validation-order regression.
#[cfg(test)]
mod tests {
    use crate::{
        commands::{Global, exec::RequestArgs},
        ctx::Ctx,
        term::Out,
    };
    use allowit_stellar_wallet as sdk;
    use std::time::Duration;
    fn vault(n: u8) -> String {
        let asset =
            sdk::asset_from_identity("VLT", Some(&sdk::account_for_seed(&[20 + n; 32]))).unwrap();
        sdk::sac_address("stellar:testnet", &asset).unwrap()
    }
    #[test]
    fn a_vault_operation_without_a_vault_or_minimum_stops_before_the_journal() {
        let dir = crate::settings::temp_dir("vault-flags");
        let global = Global {
            config_dir: Some(dir.clone()),
            rail: Some("stellar:testnet".into()),
            ..Global::default()
        };
        let mut cx = Ctx::new(global, Out::new(false, true, false, true));
        let account = sdk::account_for_seed(&[6; 32]);
        for (op, to, amount, token_out, needle) in [
            (
                "redeem",
                account.as_str(),
                "2000000",
                Some("USDC:1"),
                "vault contract",
            ),
            ("redeem", &vault(1), "2000000", None, "--token-out"),
            ("deposit", &vault(1), "USDC:1", None, "--token-out"),
            (
                "redeem",
                &vault(1),
                "USDC:1",
                Some("USDC:1"),
                "share amount",
            ),
        ] {
            let args = RequestArgs {
                op: op.into(),
                to: to.into(),
                amount: amount.into(),
                token_out: token_out.map(Into::into),
                args: None,
                request_id: Some("request-0001".into()),
                wait: Duration::ZERO,
            };
            let error = crate::commands::exec::submit(args, false, &mut cx).unwrap_err();
            assert_eq!(error.code, 2, "{op} {to}: {}", error.message);
            assert!(error.message.contains(needle), "{}", error.message);
        }
        // No journal, profile or engine was read: the config directory is empty.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }
}
