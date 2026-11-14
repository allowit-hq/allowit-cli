//! `allowit skills`: the marketplace catalog of the configured server. The
//! catalog is free to read. Its text is listing data and its prices are
//! indicative: `call` pays what the seller asks for the exact resource.
use crate::{ctx::Ctx, error::Result, term};
use allowit_sdk::marketplace::{self, CatalogEntry, Query, SearchResult};
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
    #[command(before_help = LS_EXAMPLES)]
    Ls(PageArgs),
    #[command(before_help = SHOW_EXAMPLES)]
    Show(ShowArgs),
}

const SEARCH_EXAMPLES: &str = "\
Examples:
  allowit skills search vault risk report
  allowit skills search \"weather forecast\" --limit 5 --json
  allowit skills search translate --all-rails";
const LS_EXAMPLES: &str = "\
Examples:
  allowit skills ls
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

/// Show one catalog entry: its resource, methods and indicative terms
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

fn show_page(cx: &mut Ctx, result: &SearchResult) -> Result<i32> {
    if cx.out.json {
        cx.out.json_value(&json!(result));
        return Ok(0);
    }
    let mut rows = vec![vec![
        "ENTRY".to_owned(),
        "NAME".to_owned(),
        "LISTED PRICE".to_owned(),
        "PROVIDER".to_owned(),
    ]];
    for entry in &result.entries {
        rows.push(vec![
            entry.id.clone(),
            entry.name.clone(),
            price(entry),
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
            let rows: Vec<(String, String)> = [
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
            cx.out.stdout.push_str(&term::render(&rows));
            Ok(0)
        }
    }
}

/// A search result as the MCP tools return it.
pub(crate) fn page_value(result: &SearchResult) -> Value {
    json!(result)
}
