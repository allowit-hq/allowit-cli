//! `allowit keys ls`.
use crate::{ctx::Ctx, error::Result, settings::keys};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys ls
  allowit keys ls --json";

/// List keys
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    let dir = cx.dir()?;
    let (all, invalid) = keys::list(&dir)?;
    for path in invalid {
        cx.out.progress(format!("invalid  {}", path.display()));
    }
    if cx.out.json {
        let values: Vec<_> = all.iter().map(super::view).collect();
        cx.out.json_value(&serde_json::Value::Array(values));
        return Ok(0);
    }
    let mut rows = vec![vec![
        "NAME".to_owned(),
        "RAIL".into(),
        "SIGNS".into(),
        "ADDRESS".into(),
    ]];
    for k in &all {
        rows.push(vec![
            k.name.clone(),
            k.rail.clone(),
            if k.secret { "yes" } else { "no" }.into(),
            k.address.clone(),
        ]);
    }
    if all.is_empty() {
        return Ok(0);
    }
    cx.out.table(&rows);
    Ok(0)
}
