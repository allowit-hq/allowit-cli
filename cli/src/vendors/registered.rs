//! Terminal bindings for the selected SDK capability.
//!
#[cfg(feature = "stellar")]
#[path = "stellar/approval.rs"]
pub mod approval;
#[cfg(feature = "stellar")]
#[path = "stellar/owner.rs"]
pub mod owner;
#[cfg(all(test, feature = "stellar"))]
#[path = "stellar/request.rs"]
mod request;

use super::{
    Binding, Caps, ExecRequest, Rail,
    asset::{Amount, Token},
    require_op,
};
use crate::{config::env_value, ctx::Ctx, error::Result, routing::RailInfo, settings::keys::Key};
use std::time::Duration;

pub(crate) struct Adapter(pub &'static RailInfo);

pub(crate) fn tokens_for(rail: &str) -> &'static [Token] {
    allowit_sdk_engine::execution::tokens(rail).expect("admitted rail")
}

impl Adapter {
    fn profile(
        &self,
        cx: &mut Ctx,
        wallet: Option<&str>,
    ) -> Result<allowit_sdk_engine::profiles::Profile> {
        let root = cx.dir()?;
        let selected = match wallet {
            Some(w) => Some(w.into()),
            None => cx.wallet(self.0)?,
        };
        let agent = if cx.settings()?.rail(self.0.id).agent.is_some() {
            Some(cx.agent(self.0)?.address)
        } else {
            None
        };
        allowit_sdk_engine::profiles::Cache::open(&root)?.resolve(
            &allowit_sdk_engine::profiles::Selector {
                network: self.0.id.into(),
                wallet: selected,
                agent,
                ..Default::default()
            },
        )
    }
    fn signer(&self, cx: &mut Ctx) -> Result<allowit_sdk_engine::wallet::LocalSigner> {
        let agent = cx.agent(self.0)?;
        allowit_sdk_engine::wallet::local_signer(self.0.id, &cx.signer(&agent)?)
    }
}

impl Rail for Adapter {
    fn info(&self) -> &'static RailInfo {
        self.0
    }
    fn tokens(&self) -> &'static [Token] {
        tokens_for(self.0.id)
    }
    fn caps(&self) -> Caps {
        let c =
            allowit_sdk_engine::execution::capabilities(self.0.id).expect("registered capability");
        Caps {
            curve: c.curve,
            address_is_key: c.address_is_key,
            ops: c.operations,
            exec_build_only: c.build_only,
            verbs: c.verbs,
        }
    }
    fn keys_fund(&self, cx: &mut Ctx, key: &Key) -> Result<i32> {
        if !self.0.testnet {
            return Err(self.na("keys fund"));
        }
        key.check_rail(self.0.id)?;
        let endpoint = env_value("ALLOWIT_FRIENDBOT_URL");
        let hash =
            allowit_sdk_engine::execution::fund_key(self.0.id, &key.address, Some(&endpoint))?;
        cx.out.value(&serde_json::json!({
            "key": key.name,
            "address": key.address,
            "rail": self.0.id,
            "transactionHash": hash,
        }));
        Ok(0)
    }
    fn exec_binding(&self, cx: &mut Ctx, wallet: Option<&str>, id: &str) -> Result<Binding> {
        if let Some(recovery) =
            allowit_sdk_engine::execution::retained(&cx.dir()?, self.0.id, wallet, id)?
            && recovery.signed()
        {
            let b = recovery.binding();
            return Ok(Binding {
                wallet: Some(b.wallet),
                policy: Some(b.policy),
                recovery: Some(recovery),
                ..Default::default()
            });
        }
        let authority = cx.server_authority(self.0)?;
        let p = self.profile(cx, wallet)?.with_authority(&authority)?;
        Ok(Binding {
            policy: Some(format!(
                "{}@{}",
                p.description().policy_id,
                p.description().module_digest
            )),
            wallet: Some(p.description().wallet.clone()),
            profile: Some(p),
            recovery: None,
        })
    }
    fn status_binding(&self, cx: &mut Ctx, wallet: Option<&str>, id: &str) -> Result<Binding> {
        if let Some(recovery) =
            allowit_sdk_engine::execution::retained(&cx.dir()?, self.0.id, wallet, id)?
        {
            let b = recovery.binding();
            return Ok(Binding {
                wallet: Some(b.wallet),
                policy: Some(b.policy),
                recovery: Some(recovery),
                ..Default::default()
            });
        }
        self.exec_binding(cx, wallet, id)
    }
    #[cfg(feature = "stellar")]
    fn legacy(&self, cx: &mut Ctx, verb: &str, args: &[String]) -> Result<i32> {
        match verb {
            "wallet configure" => owner::run(args, &mut cx.out.stdout),
            "wallet approve" => approval::run(args, &mut cx.out.stdout),
            _ => Err(self.na(verb)),
        }
    }
    fn wallet_show(&self, cx: &mut Ctx, wallet: Option<&str>) -> Result<i32> {
        let p = self.profile(cx, wallet)?;
        cx.out.value(&p.inspect()?);
        Ok(0)
    }
    fn policy_show(&self, cx: &mut Ctx, source: bool) -> Result<i32> {
        let p = self.profile(cx, None)?;
        if source {
            cx.out.source(&p.description().source);
        } else {
            let mut out = p.inspect()?;
            out["asset"] = out["assetSymbol"].clone();
            cx.out.value(&out);
        }
        Ok(0)
    }
    fn check_exec(
        &self,
        op: &str,
        to: &str,
        token_out: Option<&Amount>,
        args: Option<&serde_json::Value>,
    ) -> Result<()> {
        allowit_sdk_engine::execution::check(self.0.id, op, to, token_out, args.is_some())
    }
    fn exec(&self, cx: &mut Ctx, r: &ExecRequest, binding: Binding) -> Result<i32> {
        require_op(self, &r.op)?;
        allowit_sdk_engine::execution::check(
            self.0.id,
            &r.op,
            &r.to,
            r.token_out.as_ref(),
            r.args.is_some(),
        )?;
        let root = cx.dir()?;
        if let Some(recovery) = binding.recovery {
            if r.build_only {
                return Err(crate::error::Error::usage(
                    "Original signed operations support receipt recovery only",
                ));
            }
            let out = allowit_sdk_engine::execution::recover(
                recovery,
                self.signer(cx)?,
                &env_value("ALLOWIT_URL"),
                &root,
                &r.request_id,
                r.wait,
                Some(r),
            )?;
            cx.out.value(&out.document);
            return Ok(crate::exit::execution_status(out.status).code());
        }
        let p = binding.profile.ok_or_else(|| {
            crate::error::Error::config("Execution has no resolved verified profile")
        })?;
        let rpc = cx.rpc_url(self.0)?;
        if r.build_only {
            cx.out.json_value(&allowit_sdk_engine::execution::build(
                &p,
                &root,
                rpc.as_deref(),
                r,
            )?);
            return Ok(0);
        }
        let signer = self.signer(cx)?;
        crate::flush_stderr(&mut cx.out.stderr);
        let out = allowit_sdk_engine::execution::execute(p, signer, &root, rpc, r)?;
        cx.out.value(&out.document);
        Ok(crate::exit::execution_status(out.status).code())
    }
    fn status(
        &self,
        cx: &mut Ctx,
        id: &str,
        binding: Binding,
        wait: Duration,
        backend: Option<&str>,
    ) -> Result<i32> {
        let signer = self.signer(cx)?;
        let root = cx.dir()?;
        let out = if let Some(recovery) = binding.recovery {
            let selected = env_value("ALLOWIT_URL");
            allowit_sdk_engine::execution::recover(
                recovery,
                signer,
                backend.unwrap_or(&selected),
                &root,
                id,
                wait,
                None,
            )?
        } else {
            let p = binding.profile.ok_or_else(|| {
                crate::error::Error::config("Status has no resolved profile or retained operation")
            })?;
            allowit_sdk_engine::execution::status(p, signer, &root, cx.rpc_url(self.0)?, id, wait)?
        };
        cx.out.value(&out.document);
        Ok(crate::exit::execution_status(out.status).code())
    }
}

#[cfg(all(test, feature = "stellar"))]
mod tests {
    #[test]
    fn token_issuers_are_valid_strkeys() {
        for tokens in [
            super::tokens_for("stellar:testnet"),
            super::tokens_for("stellar:pubnet"),
        ] {
            for token in tokens {
                if let Some(issuer) = token.issuer {
                    allowit_stellar_wallet::account_key(issuer).unwrap();
                }
            }
        }
    }
}

#[cfg(all(test, feature = "stellar"))]
mod profile_snapshot_tests {
    use super::chain;
    use super::*;
    use crate::{commands::Global, term::Out};
    use allowit_sdk_engine::Store;
    fn seed(root: &std::path::Path, id: &str, doc: &serde_json::Value) {
        Store::open(&root.join("public-profiles/versions")).unwrap().save(id,&serde_json::json!({"schema":1,"network":"stellar:testnet","backend":"http://localhost:8080","descriptor":doc})).unwrap();
    }
    #[test]
    fn one_profile_and_authority_survive_selection_changes_before_dispatch() {
        let root = crate::settings::temp_dir("profile-snapshot");
        let selected = chain::native_selection();
        let first = chain::approved_descriptor(&selected);
        seed(&root, "first", &first);
        let mut cx = Ctx::new(
            Global {
                config_dir: Some(root.clone()),
                rail: Some("stellar:testnet".into()),
                ..Default::default()
            },
            Out::new(false, true, false, true),
        );
        cx.settings()
            .unwrap()
            .update(|s| {
                let rail = s.rail_mut("stellar:testnet");
                rail.wallet = Some(selected.config.wallet.clone());
                rail.server_authorities
                    .insert("http://localhost:8080".into(), chain::account(13));
                Ok(())
            })
            .unwrap();
        crate::config::with_settings(&[("ALLOWIT_URL", "http://localhost:8080".into())], || {
            let adapter = Adapter(crate::routing::info("stellar:testnet")?);
            let id = "snapshot-request-001";
            let binding = adapter.exec_binding(&mut cx, None, id)?;
            let mut second = first.clone();
            second["policyId"] = serde_json::json!("different-policy");
            seed(&root, "second", &second);
            let cache = allowit_sdk_engine::profiles::Cache::open(&root)?;
            let second = cache.resolve(&allowit_sdk_engine::profiles::Selector {
                network: "stellar:testnet".into(),
                wallet: Some(selected.config.wallet.clone()),
                policy_id: Some("different-policy".into()),
                ..Default::default()
            })?;
            cache.select(&allowit_sdk_engine::profiles::Selector {
                network: "stellar:testnet".into(),
                wallet: Some(selected.config.wallet.clone()),
                version: Some(second.version),
                ..Default::default()
            })?;
            drop(cache);
            cx.settings()?.update(|s| {
                s.rail_mut("stellar:testnet")
                    .server_authorities
                    .insert("http://localhost:8080".into(), chain::account(14));
                Ok(())
            })?;
            let request = ExecRequest {
                op: "transfer".into(),
                to: chain::account(15),
                amount: Amount {
                    asset: crate::vendors::asset::Asset::Installed,
                    quantity: 1,
                    decimals: None,
                    text: "1".into(),
                },
                token_out: None,
                args: None,
                request_id: id.into(),
                wait: Duration::ZERO,
                build_only: true,
                wallet: binding.wallet.clone(),
            };
            adapter.exec(&mut cx, &request, binding)?;
            let wire: serde_json::Value = serde_json::from_str(&cx.out.stdout).unwrap();
            assert_eq!(
                wire["input"]["request"]["operation"]["nonce"],
                serde_json::json!(allowit_sdk_engine::typed_request::operation_nonce(
                    &selected.policy.id,
                    id
                ))
            );
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(all(test, feature = "stellar"))]
#[path = "../../../engine/tests/support/owner_chain.rs"]
mod chain;
#[cfg(all(test, feature = "stellar"))]
#[path = "../../../engine/tests/support/owner_server.rs"]
mod wire;
