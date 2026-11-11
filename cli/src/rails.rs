//! One command adapter for every rail that the SDK registers. Commands reach
//! native behavior only through the SDK plugin registry; a verb that a rail's
//! capabilities omit exits 3.
/// Selected-rail address checks from the SDK.
pub(crate) mod address {
    pub(crate) use allowit_sdk::input::address::*;
}
/// Amount input from the SDK.
pub(crate) mod asset {
    pub(crate) use allowit_sdk::input::asset::*;
}

use crate::{
    config::env_value,
    ctx::Ctx,
    error::{Error, Result},
    exit::not_available,
    routing::RailInfo,
    settings::keys::Key,
};
use asset::{Amount, Token, Use};
use std::time::Duration;

/// Every verb a rail may serve, in `rail show` order.
pub(crate) const VERBS: &[&str] = &[
    "keys fund",
    "wallet setup",
    "wallet import",
    "wallet show",
    "policy generate",
    "policy deploy",
    "policy fund",
    "policy activate",
    "policy export",
    "policy owner-status",
    "policy show",
    "exec",
    "status",
];

/// What a rail supports. It drives `rail show`, help and the exit-3 errors.
pub(crate) struct Caps {
    pub curve: &'static str,
    pub address_is_key: bool,
    pub ops: &'static [&'static str],
    pub exec_build_only: bool,
    pub verbs: &'static [&'static str],
}

pub(crate) use allowit_sdk::execution::Request as ExecRequest;

/// What an agent request runs under: the wallet and the policy that the rail
/// resolves from the selection and its installed profile. The journal binds a
/// request ID to it, so a retry cannot run under another wallet or policy.
#[derive(Default)]
pub(crate) struct Binding {
    pub wallet: Option<String>,
    pub policy: Option<String>,
    pub profile: Option<allowit_sdk::profiles::Profile>,
    pub recovery: Option<allowit_sdk::execution::Recovery>,
}

/// The command adapter of one registered rail. Every rail uses this adapter;
/// the SDK plugin that serves the rail decides what each call does.
#[derive(Clone, Copy)]
pub(crate) struct Rail(pub &'static RailInfo);

/// The adapter for a registered rail.
pub(crate) fn rail(info: &'static RailInfo) -> Rail {
    Rail(info)
}

/// Refuses an operation other than those the rail lists.
pub(crate) fn require_op(rail: &Rail, op: &str) -> Result<()> {
    if rail.caps().ops.contains(&op) {
        Ok(())
    } else {
        Err(not_available(&format!("exec --op {op}"), rail.info().id))
    }
}

impl Rail {
    fn profile(
        &self,
        cx: &mut Ctx,
        wallet: Option<&str>,
    ) -> Result<allowit_sdk::profiles::Profile> {
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
        allowit_sdk::profiles::Cache::open(&root)?.resolve(&allowit_sdk::profiles::Selector {
            network: self.0.id.into(),
            wallet: selected,
            agent,
            ..Default::default()
        })
    }
    fn signer(&self, cx: &mut Ctx) -> Result<allowit_sdk::wallet::LocalSigner> {
        let agent = cx.agent(self.0)?;
        allowit_sdk::wallet::local_signer(self.0.id, &cx.signer(&agent)?)
    }
    pub(crate) fn parse_amount(&self, text: &str, usage: Use) -> Result<Amount> {
        asset::parse(self.info().family, self.tokens(), text, usage)
    }
    pub(crate) fn na(&self, verb: &str) -> Error {
        not_available(verb, self.info().id)
    }
    pub(crate) fn info(&self) -> &'static RailInfo {
        self.0
    }
    pub(crate) fn tokens(&self) -> &'static [Token] {
        allowit_sdk::execution::tokens(self.0.id).expect("registered rail")
    }
    pub(crate) fn caps(&self) -> Caps {
        let c = allowit_sdk::execution::capabilities(self.0.id).expect("registered capability");
        Caps {
            curve: c.curve,
            address_is_key: c.address_is_key,
            ops: c.operations,
            exec_build_only: c.build_only,
            verbs: c.verbs,
        }
    }
    pub(crate) fn keys_fund(&self, cx: &mut Ctx, key: &Key) -> Result<i32> {
        if !self.0.testnet {
            return Err(self.na("keys fund"));
        }
        key.check_rail(self.0.id)?;
        let endpoint = env_value("ALLOWIT_FAUCET_URL");
        let hash = allowit_sdk::execution::fund_key(self.0.id, &key.address, Some(&endpoint))?;
        cx.out.value(&serde_json::json!({
            "key": key.name,
            "address": key.address,
            "rail": self.0.id,
            "transactionHash": hash,
        }));
        Ok(0)
    }
    pub(crate) fn exec_binding(
        &self,
        cx: &mut Ctx,
        wallet: Option<&str>,
        id: &str,
    ) -> Result<Binding> {
        if let Some(recovery) = allowit_sdk::execution::retained(&cx.dir()?, self.0.id, wallet, id)?
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
    pub(crate) fn status_binding(
        &self,
        cx: &mut Ctx,
        wallet: Option<&str>,
        id: &str,
    ) -> Result<Binding> {
        if let Some(recovery) = allowit_sdk::execution::retained(&cx.dir()?, self.0.id, wallet, id)?
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
    /// The public description of a profile with the wallet kind that it
    /// binds, when the rail reports one, and its params as an owner reads
    /// them. Returns the JSON document and the document that text prints.
    ///
    /// JSON keeps `params` as stored and adds `paramsDisplay`: the SDK's
    /// line of each param, with its label and its value in the unit of the
    /// installed asset. Text prints that value in place of the stored one,
    /// so an amount never appears as bare base units. A profile whose params
    /// the rail cannot read prints the stored values, never a placeholder.
    fn inspect(
        &self,
        p: &allowit_sdk::profiles::Profile,
    ) -> Result<(serde_json::Value, serde_json::Value)> {
        let mut out = p.inspect()?;
        if out.get(crate::kinds::KEY).is_none()
            && let Ok(document) = serde_json::to_vec(&p.document()?)
            && let Ok(kind) = allowit_sdk::kinds::wallet_kind(self.0.id, &document)
        {
            out[crate::kinds::KEY] = kind.id().into();
        }
        let mut text = out.clone();
        if let Ok(lines) = allowit_sdk::kinds::profile_param_lines(p)
            && !lines.is_empty()
            && let Ok(display) = serde_json::to_value(&lines)
        {
            text["params"] = lines
                .into_iter()
                .map(|line| (line.key, line.value.into()))
                .collect::<serde_json::Map<_, _>>()
                .into();
            out["paramsDisplay"] = display;
        }
        Ok((out, text))
    }
    /// The spending limits of the profile's wallet: what each limit allows,
    /// what the policy has spent, fees included, and what remains. The read
    /// needs the selected agent key and the trusted server, as `status` does;
    /// without an agent key it is not attempted. `Ok(None)` means that the
    /// rail holds no report for this wallet yet.
    fn limits(
        &self,
        cx: &mut Ctx,
        wallet: Option<&str>,
    ) -> Option<Result<Option<Vec<allowit_sdk::execution::LimitReport>>>> {
        if !self.caps().verbs.contains(&"status")
            || cx.settings().ok()?.rail(self.0.id).agent.is_none()
        {
            return None;
        }
        let mut read = || {
            let authority = cx.server_authority(self.0)?;
            let p = self.profile(cx, wallet)?.with_authority(&authority)?;
            let signer = self.signer(cx)?;
            let root = cx.dir()?;
            let rpc = cx.rpc_url(self.0)?;
            crate::flush_stderr(&mut cx.out.stderr);
            allowit_sdk::execution::limits(p, signer, &root, rpc)
        };
        Some(read())
    }
    /// The param lines of a profile, from the SDK: the source of the limits
    /// that have no counter. A profile whose params the rail cannot read has
    /// none.
    fn param_lines(p: &allowit_sdk::profiles::Profile) -> Vec<allowit_sdk::kinds::ParamLine> {
        allowit_sdk::kinds::profile_param_lines(p).unwrap_or_default()
    }
    /// The param lines of the installed profile of a retained request, which
    /// carries no profile of its own. They are shown only when that profile
    /// still names the policy that sent the request.
    fn retained_param_lines(
        &self,
        cx: &mut Ctx,
        binding: &Binding,
    ) -> Vec<allowit_sdk::kinds::ParamLine> {
        let Ok(p) = self.profile(cx, binding.wallet.as_deref()) else {
            return vec![];
        };
        let policy = format!(
            "{}@{}",
            p.description().policy_id,
            p.description().module_digest
        );
        if binding.policy.as_deref() == Some(policy.as_str()) {
            Self::param_lines(&p)
        } else {
            vec![]
        }
    }
    fn show(cx: &mut Ctx, json: &serde_json::Value, text: &serde_json::Value) {
        if cx.out.json {
            cx.out.json_value(json);
        } else {
            cx.out.text(text);
        }
    }
    pub(crate) fn wallet_show(&self, cx: &mut Ctx, wallet: Option<&str>) -> Result<i32> {
        use crate::receipt::{UNCOUNTED_LIMITS, limits_table, uncounted_limits};
        use allowit_sdk::execution::Limits;
        let p = self.profile(cx, wallet)?;
        let (mut json, mut text) = self.inspect(&p)?;
        let lines = Self::param_lines(&p);
        // A limit without a counter comes from the verified profile, so it
        // shows with or without a report of the counted limits.
        let mut reported = vec![];
        match self.limits(cx, wallet) {
            Some(Ok(Some(limits))) => {
                json[Limits::KEY] = serde_json::to_value(&limits)
                    .map_err(|_| Error::config("Cannot encode the spending limits"))?;
                reported = limits;
            }
            // No report yet: say so, and never print a zero or an empty table.
            Some(Ok(None)) => {
                json[Limits::KEY] = serde_json::Value::Null;
                text[Limits::KEY] =
                    "not reported yet: this device holds no request of this wallet".into();
            }
            // The wallet is shown from the verified profile; the limits are a second read.
            Some(Err(error)) => cx.out.warning(format!(
                "the spending limits were not read: {}",
                error.message
            )),
            None => {}
        }
        let uncounted = uncounted_limits(&lines, &reported);
        if !uncounted.is_empty() {
            json[UNCOUNTED_LIMITS] = serde_json::to_value(&uncounted)
                .map_err(|_| Error::config("Cannot encode the limits"))?;
        }
        let table = limits_table(&reported, &uncounted);
        Self::show(cx, &json, &text);
        if !cx.out.json && !table.is_empty() {
            cx.out.line("");
            cx.out.table(&table);
        }
        Ok(0)
    }
    pub(crate) fn policy_show(&self, cx: &mut Ctx, source: bool) -> Result<i32> {
        let p = self.profile(cx, None)?;
        if source {
            cx.out.source(&p.description().source);
        } else {
            let (mut json, mut text) = self.inspect(&p)?;
            for out in [&mut json, &mut text] {
                out["asset"] = out["assetSymbol"].clone();
            }
            Self::show(cx, &json, &text);
        }
        Ok(0)
    }
    pub(crate) fn check_exec(
        &self,
        op: &str,
        to: &str,
        token_out: Option<&Amount>,
        args: Option<&serde_json::Value>,
    ) -> Result<()> {
        allowit_sdk::execution::check(self.0.id, op, to, token_out, args.is_some())
    }
    pub(crate) fn exec(&self, cx: &mut Ctx, r: &ExecRequest, binding: Binding) -> Result<i32> {
        let root = cx.dir()?;
        if let Some(recovery) = binding.recovery {
            if r.build_only {
                return Err(crate::error::Error::usage(
                    "Original signed operations support receipt recovery only",
                ));
            }
            // A retained operation holds its own server origin; ALLOWIT_URL
            // serves only a record without one, so it may be unset here.
            let server = env_value("ALLOWIT_URL");
            let signer = self.signer(cx)?;
            let lines = self.retained_param_lines(
                cx,
                &Binding {
                    wallet: binding.wallet,
                    policy: binding.policy,
                    ..Default::default()
                },
            );
            crate::flush_stderr(&mut cx.out.stderr);
            let out = allowit_sdk::execution::recover(
                recovery,
                signer,
                &server,
                &root,
                &r.request_id,
                r.wait,
                Some(r),
            )?;
            crate::receipt::show(&mut cx.out, self.0.id, out.document, &lines);
            return Ok(crate::exit::execution_status(out.status).code());
        }
        let p = binding.profile.ok_or_else(|| {
            crate::error::Error::config("Execution has no resolved verified profile")
        })?;
        let rpc = cx.rpc_url(self.0)?;
        if r.build_only {
            cx.out.value(&allowit_sdk::execution::build(
                &p,
                &root,
                rpc.as_deref(),
                r,
            )?);
            return Ok(0);
        }
        let signer = self.signer(cx)?;
        let lines = Self::param_lines(&p);
        crate::flush_stderr(&mut cx.out.stderr);
        let out = allowit_sdk::execution::execute(p, signer, &root, rpc, r)?;
        crate::receipt::show(&mut cx.out, self.0.id, out.document, &lines);
        Ok(crate::exit::execution_status(out.status).code())
    }
    pub(crate) fn status(
        &self,
        cx: &mut Ctx,
        id: &str,
        binding: Binding,
        wait: Duration,
        backend: Option<&str>,
    ) -> Result<i32> {
        let signer = self.signer(cx)?;
        let root = cx.dir()?;
        let lines = match &binding.profile {
            Some(p) => Self::param_lines(p),
            None => self.retained_param_lines(cx, &binding),
        };
        let out = if let Some(recovery) = binding.recovery {
            // The journal's origin serves a retained request. ALLOWIT_URL is
            // the fallback for a record without one, and may be unset.
            let server = match backend {
                Some(origin) => origin.to_owned(),
                None => env_value("ALLOWIT_URL"),
            };
            crate::flush_stderr(&mut cx.out.stderr);
            allowit_sdk::execution::recover(recovery, signer, &server, &root, id, wait, None)?
        } else {
            let p = binding.profile.ok_or_else(|| {
                crate::error::Error::config("Status has no resolved profile or retained operation")
            })?;
            let rpc = cx.rpc_url(self.0)?;
            crate::flush_stderr(&mut cx.out.stderr);
            allowit_sdk::execution::status(p, signer, &root, rpc, id, wait)?
        };
        crate::receipt::show(&mut cx.out, self.0.id, out.document, &lines);
        Ok(crate::exit::execution_status(out.status).code())
    }
}
