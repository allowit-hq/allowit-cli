//! `allowit history`: the local request journal, newest first.
use crate::{ctx::Ctx, error::Result, settings::journal};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit history
  allowit history --limit 5 --json";

/// List recent requests from the local journal
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Show at most N requests
    #[arg(long, value_name = "N", default_value_t = 20)]
    pub limit: usize,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let dir = cx.dir()?;
    let mut entries = journal::entries(&dir)?;
    entries.truncate(args.limit);
    if cx.out.json {
        cx.out
            .json_value(&serde_json::to_value(&entries).unwrap_or_default());
        return Ok(0);
    }
    if entries.is_empty() {
        return Ok(0);
    }
    let mut rows = vec![vec![
        "REQUEST".to_owned(),
        "COMMAND".into(),
        "RAIL".into(),
        "OP".into(),
        "AMOUNT".into(),
        "EXIT".into(),
        "TO".into(),
    ]];
    for e in entries {
        rows.push(vec![
            e.request_id,
            e.command,
            e.rail,
            e.op,
            e.amount,
            e.exit.map_or_else(|| "-".into(), |c| c.to_string()),
            e.to,
        ]);
    }
    cx.out.table(&rows);
    Ok(0)
}
