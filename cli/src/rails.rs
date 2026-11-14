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
    /// The limits of the profile's wallet from the SDK: what each counted
    /// limit allows, what the policy has spent, fees included, and what
    /// remains, with the per-payment limits. The read needs the selected
    /// agent key and the trusted server, as `status` does; without an agent
    /// key it is not attempted. A wallet that has executed nothing reports
    /// its limits.
    fn limits(
        &self,
        cx: &mut Ctx,
        wallet: Option<&str>,
    ) -> Option<Result<allowit_sdk::execution::WalletLimits>> {
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
            allowit_sdk::execution::wallet_limits(p, signer, &root, rpc)
        };
        Some(read())
    }
    /// The per-payment limits of a profile, from the SDK. A profile whose
    /// params the rail cannot read has none.
    fn payment_limits(
        p: &allowit_sdk::profiles::Profile,
    ) -> Vec<allowit_sdk::execution::PaymentLimit> {
        allowit_sdk::execution::payment_limits(p).unwrap_or_default()
    }
    /// The per-payment limits of the installed profile of a retained request,
    /// which carries no profile of its own. They are shown only when that
    /// profile still names the policy that sent the request.
    fn retained_payment_limits(
        &self,
        cx: &mut Ctx,
        binding: &Binding,
    ) -> Vec<allowit_sdk::execution::PaymentLimit> {
        let Ok(p) = self.profile(cx, binding.wallet.as_deref()) else {
            return vec![];
        };
        let policy = format!(
            "{}@{}",
            p.description().policy_id,
            p.description().module_digest
        );
        if binding.policy.as_deref() == Some(policy.as_str()) {
            Self::payment_limits(&p)
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
        use crate::receipt::limits_table;
        use allowit_sdk::execution::{Limits, WalletLimits};
        let p = self.profile(cx, wallet)?;
        let (mut json, mut text) = self.inspect(&p)?;
        // A per-payment limit comes from the verified profile, so it shows
        // with or without a report of the counted limits.
        let mut limits = WalletLimits {
            limits: None,
            payment_limits: Self::payment_limits(&p),
        };
        match self.limits(cx, wallet) {
            Some(Ok(read)) => {
                limits = read;
                match &limits.limits {
                    Some(counted) => {
                        json[Limits::KEY] = serde_json::to_value(counted)
                            .map_err(|_| Error::config("Cannot encode the spending limits"))?;
                    }
                    // No counters: say so, and never print a zero or an empty table.
                    None => {
                        json[Limits::KEY] = serde_json::Value::Null;
                        text[Limits::KEY] =
                            "spent and remaining not reported: the server did not read the counters"
                                .into();
                    }
                }
            }
            // The wallet is shown from the verified profile; the limits are a
            // second read. A typed refusal shows the server's code, and then
            // no limit shows: the server reports none, for example when the
            // wallet enforces other params than the profile holds.
            Some(Err(error)) => {
                if error.api_code.is_some() {
                    limits.payment_limits.clear();
                }
                cx.out.warning(match &error.api_code {
                    Some(code) => format!(
                        "the spending limits were not read: {code}: {}",
                        error.message
                    ),
                    None => format!("the spending limits were not read: {}", error.message),
                });
            }
            None => {}
        }
        if !limits.payment_limits.is_empty() {
            json[WalletLimits::PAYMENT_KEY] = serde_json::to_value(&limits.payment_limits)
                .map_err(|_| Error::config("Cannot encode the limits"))?;
        }
        let table = limits_table(
            limits.limits.as_deref().unwrap_or_default(),
            &limits.payment_limits,
        );
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
        if r.build_only && binding.recovery.is_none() {
            let root = cx.dir()?;
            let p = binding.profile.ok_or_else(|| {
                crate::error::Error::config("Execution has no resolved verified profile")
            })?;
            let rpc = cx.rpc_url(self.0)?;
            cx.out.value(&allowit_sdk::execution::build(
                &p,
                &root,
                rpc.as_deref(),
                r,
            )?);
            return Ok(0);
        }
        let (out, payment) = self.exec_outcome(cx, r, binding)?;
        crate::receipt::show(&mut cx.out, self.0.id, out.document, &payment);
        Ok(crate::exit::execution_status(out.status).code())
    }
    /// Sends one request and returns its outcome with the per-payment limits
    /// of its profile, and prints nothing. `exec` prints the receipt; a paid
    /// call reads it to build the proof.
    pub(crate) fn exec_outcome(
        &self,
        cx: &mut Ctx,
        r: &ExecRequest,
        binding: Binding,
    ) -> Result<(
        allowit_sdk::execution::Outcome,
        Vec<allowit_sdk::execution::PaymentLimit>,
    )> {
        let root = cx.dir()?;
        if let Some(recovery) = binding.recovery {
            if r.build_only {
                return Err(crate::error::Error::usage(
                    "Original signed operations support receipt recovery only",
                ));
            }
            // A retained operation holds its own server origin; ALLOWIT_URL,
            // or the production server, serves only a record without one.
            let server = cx.server_url()?;
            let signer = self.signer(cx)?;
            let payment = self.retained_payment_limits(
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
            return Ok((out, payment));
        }
        let p = binding.profile.ok_or_else(|| {
            crate::error::Error::config("Execution has no resolved verified profile")
        })?;
        let rpc = cx.rpc_url(self.0)?;
        if r.build_only {
            return Err(crate::error::Error::usage(
                "A built request has no outcome to read",
            ));
        }
        let signer = self.signer(cx)?;
        let payment = Self::payment_limits(&p);
        crate::flush_stderr(&mut cx.out.stderr);
        let out = allowit_sdk::execution::execute(p, signer, &root, rpc, r)?;
        Ok((out, payment))
    }
    pub(crate) fn status(
        &self,
        cx: &mut Ctx,
        id: &str,
        binding: Binding,
        wait: Duration,
        backend: Option<&str>,
    ) -> Result<i32> {
        let (out, payment) = self.status_outcome(cx, id, binding, wait, backend)?;
        crate::receipt::show(&mut cx.out, self.0.id, out.document, &payment);
        Ok(crate::exit::execution_status(out.status).code())
    }
    /// Reads one request and returns its outcome, and prints nothing.
    pub(crate) fn status_outcome(
        &self,
        cx: &mut Ctx,
        id: &str,
        binding: Binding,
        wait: Duration,
        backend: Option<&str>,
    ) -> Result<(
        allowit_sdk::execution::Outcome,
        Vec<allowit_sdk::execution::PaymentLimit>,
    )> {
        let signer = self.signer(cx)?;
        let root = cx.dir()?;
        let payment = match &binding.profile {
            Some(p) => Self::payment_limits(p),
            None => self.retained_payment_limits(cx, &binding),
        };
        let out = if let Some(recovery) = binding.recovery {
            // The journal's origin serves a retained request. ALLOWIT_URL, or
            // the production server, is the fallback for a record without one.
            let server = match backend {
                Some(origin) => origin.to_owned(),
                None => cx.server_url()?,
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
        Ok((out, payment))
    }
}
