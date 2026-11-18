//! `allowit skills`: the marketplace catalog of the configured server. The
//! catalog is free to read. Its listing text is data and its prices are
//! indicative: `call` pays what the seller asks for the exact resource. What
//! Allowit observed of an entry, and the owners' reputation labels, are shown
//! apart from the listing, each with where it came from.
use crate::{ctx::Ctx, error::Result, term};
use allowit_sdk::marketplace::{
    self, AccessKind, CatalogEntry, Label, ObservedFact, ObservedOrigin, Populations, Query,
    SearchResult,
};
use serde_json::{Value, json};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit skills search vault risk report
  allowit skills search \"weather forecast\" --limit 5 --json
  allowit skills ls
  allowit skills show ENTRY";

/// Find paid services in the server's marketplace catalog
#[derive(clap::Subcommand, Debug)]
pub(crate) enum Command {
    #[command(before_help = SEARCH_EXAMPLES)]
    Search(SearchArgs),
    /// List the catalog in entry order
    #[command(before_help = LS_EXAMPLES)]
    Ls(PageArgs),
    #[command(before_help = SHOW_EXAMPLES)]
    Show(ShowArgs),
}

const SEARCH_EXAMPLES: &str = "\
Examples:
  allowit skills search vault risk report
  allowit skills search \"weather forecast\" --limit 5 --json
  allowit skills search translate --all-rails
  allowit skills search token price --access free
  allowit skills search audit --capability security --asset USDC";
const LS_EXAMPLES: &str = "\
Examples:
  allowit skills ls
  allowit skills ls --access proof-of-work
  allowit skills ls --limit 50 --cursor 50 --json";
const SHOW_EXAMPLES: &str = "\
Examples:
  allowit skills show ENTRY
  allowit skills show ENTRY --json";

/// The page flags shared by `search` and `ls`.
#[derive(clap::Args, Debug, Default)]
pub(crate) struct PageArgs {
    /// Entries in one page, from 1 to 50
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
    /// The next cursor that the previous page printed
    #[arg(long, value_name = "CURSOR")]
    pub cursor: Option<String>,
    /// List entries of every rail, not only of the selected one
    #[arg(long)]
    pub all_rails: bool,
    /// Keep entries whose listing names this capability, such as finance
    #[arg(long, value_name = "CAPABILITY")]
    pub capability: Option<String>,
    /// Keep entries that offer this way to get them
    #[arg(long, value_enum, value_name = "KIND")]
    pub access: Option<Access>,
    /// Keep entries that accept this asset reference
    #[arg(long, value_name = "ASSET")]
    pub asset: Option<String>,
}

/// A way to get a resource.
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub(crate) enum Access {
    /// A payment of the seller's challenge
    Payment,
    /// No payment and no condition
    Free,
    /// No payment within the seller's quota
    FreeQuota,
    /// No payment, but a zero-price signature
    Signature,
    /// No payment, but a proof of work
    ProofOfWork,
}
impl Access {
    fn kind(self) -> AccessKind {
        match self {
            Self::Payment => AccessKind::Payment,
            Self::Free => AccessKind::Free,
            Self::FreeQuota => AccessKind::FreeQuota,
            Self::Signature => AccessKind::Signature,
            Self::ProofOfWork => AccessKind::ProofOfWork,
        }
    }
}

/// Search the catalog of the ALLOWIT_URL server by meaning and keywords
#[derive(clap::Args, Debug)]
pub(crate) struct SearchArgs {
    /// What the service should do
    #[arg(required = true, value_name = "QUERY")]
    pub query: Vec<String>,
    #[command(flatten)]
    pub page: PageArgs,
}

/// Show one catalog entry: its listing, its access, what Allowit observed and its reputation
#[derive(clap::Args, Debug)]
pub(crate) struct ShowArgs {
    /// The entry ID that search or ls printed
    #[arg(value_name = "ENTRY")]
    pub id: String,
}

/// Search the catalog of the configured server. The selected rail, when one
/// is selected, keeps the entries whose seller accepts payment on it.
pub(crate) fn search(cx: &mut Ctx, text: &str, page: &PageArgs) -> Result<SearchResult> {
    let server = cx.server_url()?;
    let network = if page.all_rails {
        None
    } else {
        cx.rail_opt()?.map(|rail| rail.id.to_owned())
    };
    crate::flush_stderr(&mut cx.out.stderr);
    marketplace::search(
        &server,
        &Query {
            text: text.to_owned(),
            network,
            asset: page.asset.clone(),
            access: page.access.map(Access::kind),
            capability: page.capability.clone(),
            limit: page.limit,
            cursor: page.cursor.clone(),
        },
    )
}

/// Read one catalog entry of the configured server.
pub(crate) fn entry(cx: &mut Ctx, id: &str) -> Result<CatalogEntry> {
    let server = cx.server_url()?;
    crate::flush_stderr(&mut cx.out.stderr);
    marketplace::entry(&server, id)
}

fn price(entry: &CatalogEntry) -> String {
    entry.terms.as_ref().map_or_else(
        || "-".to_owned(),
        |terms| format!("{} {}", terms.price, terms.unit),
    )
}

/// The ways to get a resource, such as `payment, proof of work`.
fn access(entry: &CatalogEntry) -> String {
    if entry.access.is_empty() {
        return "-".to_owned();
    }
    entry
        .access
        .iter()
        .map(|access| kind_name(access.kind))
        .collect::<Vec<_>>()
        .join(", ")
}
fn kind_name(kind: AccessKind) -> &'static str {
    match kind {
        AccessKind::Payment => "payment",
        AccessKind::Free => "free",
        AccessKind::FreeQuota => "free quota",
        AccessKind::Signature => "zero-price signature",
        AccessKind::ProofOfWork => "proof of work",
    }
}
/// One label in words: unknown without support, else its support and scores.
fn label(label: &Label, who: &str) -> String {
    if label.is_unknown() {
        return format!("{who} unknown");
    }
    format!(
        "{who} {} (good {}, bad {})",
        label.support,
        label.good.unwrap_or(0),
        label.bad.unwrap_or(0)
    )
}
/// The populations of one signal, closest first.
fn populations(populations: &Populations) -> String {
    let mut parts = Vec::new();
    if let Some(policy) = &populations.policy {
        parts.push(label(policy, "this policy"));
    }
    if let Some(owner) = &populations.owner {
        parts.push(label(owner, "your other policies"));
    }
    parts.push(label(&populations.others, "other owners"));
    parts.join(" · ")
}

fn show_page(cx: &mut Ctx, result: &SearchResult) -> Result<i32> {
    if cx.out.json {
        cx.out.json_value(&json!(result));
        return Ok(0);
    }
    let mut rows = vec![vec![
        "ENTRY".to_owned(),
        "NAME".to_owned(),
        "LISTED PRICE".to_owned(),
        "ACCESS".to_owned(),
        "PROVIDER".to_owned(),
    ]];
    for entry in &result.entries {
        rows.push(vec![
            entry.id.clone(),
            entry.name.clone(),
            price(entry),
            access(entry),
            entry.provider.clone(),
        ]);
    }
    if result.entries.is_empty() {
        cx.out.progress("no entry matches");
    } else {
        cx.out.table(&rows);
    }
    let ranking = serde_json::to_value(result.ranking)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    cx.out.progress(format!("ranking  {ranking}"));
    if let Some(cursor) = &result.next_cursor {
        cx.out.progress(format!("next page  --cursor {cursor}"));
    }
    Ok(0)
}

pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Search(args) => {
            let result = search(cx, &args.query.join(" "), &args.page)?;
            show_page(cx, &result)
        }
        Command::Ls(page) => {
            let result = search(cx, "", &page)?;
            show_page(cx, &result)
        }
        Command::Show(args) => {
            let entry = entry(cx, &args.id)?;
            if cx.out.json {
                cx.out.json_value(&json!(entry));
                return Ok(0);
            }
            let list = |values: &[String]| {
                if values.is_empty() {
                    "-".to_owned()
                } else {
                    values.join(", ")
                }
            };
            let mut rows: Vec<(String, String)> = [
                ("entry", entry.id.clone()),
                ("name", entry.name.clone()),
                ("provider", entry.provider.clone()),
                ("description", entry.description.clone()),
                ("resource", entry.resource.clone()),
                ("methods", list(&entry.methods)),
                ("listedPrice", price(&entry)),
                ("networks", list(&entry.networks)),
                ("assets", list(&entry.assets)),
                ("tags", list(&entry.tags)),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
            let mut row = |key: &str, value: String| rows.push((key.to_owned(), value));
            if let Some(source) = &entry.source {
                if let Some(listing) = &source.listing {
                    row("listedAt", listing.clone());
                }
                if let Some(captured) = &source.captured {
                    row("capturedIn", captured.clone());
                }
            }
            if let Some(operation) = entry.operation {
                let operation = serde_json::to_value(operation)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default();
                row("operation", operation);
            }
            for access in &entry.access {
                let terms: Vec<&str> = [
                    &access.protocol,
                    &access.scheme,
                    &access.network,
                    &access.asset,
                ]
                .into_iter()
                .flatten()
                .map(String::as_str)
                .chain(access.detail.as_deref())
                .collect();
                let name = kind_name(access.kind);
                row(
                    "access",
                    if terms.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{name}: {}", terms.join(" "))
                    },
                );
            }
            for fact in &entry.observed {
                let who = match fact.origin {
                    ObservedOrigin::Registry => "the registry check",
                    ObservedOrigin::Server => "this server",
                };
                let offer: Vec<&str> = [&fact.scheme, &fact.amount, &fact.asset]
                    .into_iter()
                    .flatten()
                    .map(String::as_str)
                    .collect();
                let text = match fact.fact {
                    ObservedFact::Response => format!(
                        "HTTP {} read by {who} on {}{}{}",
                        fact.status
                            .map_or_else(|| "-".to_owned(), |s| s.to_string()),
                        fact.at,
                        if offer.is_empty() {
                            String::new()
                        } else {
                            format!(": offer {}", offer.join(" "))
                        },
                        match fact.payable {
                            Some(false) => " (needs correction)",
                            _ => "",
                        }
                    ),
                    ObservedFact::Settlement => format!(
                        "{} payment(s) settled by {who}, the latest on {}{}",
                        fact.count.unwrap_or(0),
                        fact.at,
                        if offer.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", offer.join(" "))
                        }
                    ),
                };
                row("observed", text);
            }
            if let Some(labels) = &entry.reputation {
                row("suitability", populations(&labels.suitability));
                row("delivery", populations(&labels.delivery));
                row("quality", populations(&labels.quality));
            }
            cx.out.stdout.push_str(&term::render(&rows));
            Ok(0)
        }
    }
}

/// A search result as the MCP tools return it.
pub(crate) fn page_value(result: &SearchResult) -> Value {
    json!(result)
}
