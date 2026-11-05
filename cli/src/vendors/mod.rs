//! Command adapters over registered SDK capabilities.
//! Commands reach native behavior through the Rail trait; unsupported capabilities exit 3.
pub(crate) mod address;
pub(crate) mod asset;
pub(crate) mod registered;
#[cfg(feature = "stellar")]
pub mod stellar;

use crate::{
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

pub(crate) use allowit_sdk_engine::execution::Request as ExecRequest;

/// What an agent request runs under: the wallet and the policy that the rail
/// resolves from the selection and its installed profile. The journal binds a
/// request ID to it, so a retry cannot run under another wallet or policy.
#[derive(Default)]
pub(crate) struct Binding {
    pub wallet: Option<String>,
    pub policy: Option<String>,
    pub profile: Option<allowit_sdk_engine::profiles::Profile>,
    pub recovery: Option<allowit_sdk_engine::execution::Recovery>,
}

/// A rail adapter. Defaults exit 3: the verb is not available on this rail.
pub(crate) trait Rail {
    fn info(&self) -> &'static RailInfo;
    fn tokens(&self) -> &'static [Token];
    fn caps(&self) -> Caps;
    fn parse_amount(&self, text: &str, usage: Use) -> Result<Amount> {
        asset::parse(self.info().family, self.tokens(), text, usage)
    }
    fn na(&self, verb: &str) -> Error {
        not_available(verb, self.info().id)
    }
    /// Checks the operation flags of an `exec` request before any
    /// settings, journal or network access.
    fn check_exec(
        &self,
        _op: &str,
        _to: &str,
        _token_out: Option<&Amount>,
        _args: Option<&serde_json::Value>,
    ) -> Result<()> {
        Ok(())
    }
    /// Resolves the wallet and policy that an agent request runs under, from
    /// the selected wallet (if any) and the rail's installed profile. Refuses
    /// a selection that the request path cannot confirm. Runs before a
    /// request ID is spent; `status` also runs it to check the journal.
    fn exec_binding(&self, _: &mut Ctx, wallet: Option<&str>, _id: &str) -> Result<Binding> {
        Ok(Binding {
            wallet: wallet.map(str::to_owned),
            policy: None,
            ..Default::default()
        })
    }
    fn status_binding(&self, cx: &mut Ctx, wallet: Option<&str>, _id: &str) -> Result<Binding> {
        self.exec_binding(cx, wallet, _id)
    }
    /// Settings that every current command on this rail runs with, in place
    /// of environment values that the user did not set.
    fn settings(&self, _: &mut Ctx) -> Result<Vec<(&'static str, String)>> {
        Ok(vec![])
    }
    /// Builds a policy source for this rail: the generated Rust source.
    /// A hidden command of earlier releases: VERB names it, ARGS are its words.
    fn legacy(&self, _: &mut Ctx, verb: &str, _: &[String]) -> Result<i32> {
        Err(self.na(verb))
    }
    fn keys_fund(&self, _: &mut Ctx, _: &Key) -> Result<i32> {
        Err(self.na("keys fund"))
    }
    /// Imports an executor bundle (a JSON object) under a key name.
    fn wallet_show(&self, _: &mut Ctx, _: Option<&str>) -> Result<i32> {
        Err(self.na("wallet show"))
    }
    fn policy_show(&self, _: &mut Ctx, _: bool) -> Result<i32> {
        Err(self.na("policy show"))
    }
    fn exec(&self, _: &mut Ctx, _: &ExecRequest, _binding: Binding) -> Result<i32> {
        Err(self.na("exec"))
    }
    fn status(
        &self,
        _: &mut Ctx,
        _: &str,
        _binding: Binding,
        _: Duration,
        _backend: Option<&str>,
    ) -> Result<i32> {
        Err(self.na("status"))
    }
}

/// The adapter for an admitted Stellar rail.
pub(crate) fn rail(info: &'static RailInfo) -> Box<dyn Rail> {
    Box::new(registered::Adapter(info))
}

/// Refuses an operation other than those the rail lists.
pub(crate) fn require_op(rail: &dyn Rail, op: &str) -> Result<()> {
    if rail.caps().ops.contains(&op) {
        Ok(())
    } else {
        Err(not_available(&format!("exec --op {op}"), rail.info().id))
    }
}
