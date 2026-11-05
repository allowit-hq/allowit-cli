//! Concrete rail examples use the effective local selection, then build order.
use super::Global;
use crate::{routing, settings::Settings};
use clap::{CommandFactory, FromArgMatches};

const RAIL_MARKER: &str = "__ALLOWIT_EXAMPLE_RAIL__";

/// Read only the ordinary global options, allowing incomplete help commands.
/// A boolean help flag prevents this preliminary parser from exiting early.
fn help_globals(argv: &[String]) -> Global {
    fn without_help(cmd: clap::Command) -> clap::Command {
        cmd.disable_help_flag(true)
            .disable_help_subcommand(true)
            .mut_subcommands(without_help)
    }
    let parser = without_help(super::Cli::command()).ignore_errors(true).arg(
        clap::Arg::new("example_help")
            .long("help")
            .short('h')
            .global(true)
            .action(clap::ArgAction::SetTrue),
    );
    match parser.try_get_matches_from(argv) {
        Ok(mut matches) => Global::from_arg_matches_mut(&mut matches).unwrap_or_default(),
        Err(_) => Global::default(),
    }
}

fn selected(global: &Global) -> Option<&'static str> {
    let configured;
    let candidate = match global.rail.as_deref() {
        Some(rail) => Some(rail),
        None => {
            configured = global
                .config_dir
                .clone()
                .or_else(|| crate::settings::default_dir().ok())
                .and_then(|dir| Settings::load(dir).ok())
                .and_then(|settings| settings.config.rail);
            configured.as_deref()
        }
    };
    candidate
        .and_then(|rail| routing::info(rail).ok())
        .map(|rail| rail.id)
}

fn bind(text: &str, rail: Option<&str>) -> String {
    text.split("\n\n")
        .filter_map(|block| {
            let lines: Vec<_> = block
                .lines()
                .filter_map(|line| match rail {
                    Some(rail) => Some(line.replace(RAIL_MARKER, rail)),
                    None if line.contains(RAIL_MARKER) => None,
                    None => Some(line.to_owned()),
                })
                .collect();
            let value = lines.join("\n");
            (value != "Examples:").then_some(value)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn render(text: &str, global: &Global) -> String {
    let rail = selected(global).or_else(|| routing::all().first().map(|rail| rail.id));
    let mut rendered = bind(text, rail);
    if text.ends_with('\n') && !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

pub(super) fn help(text: String, argv: &[String]) -> String {
    if !text.contains(RAIL_MARKER) {
        return text;
    }
    render(&text, &help_globals(argv))
}

#[cfg(test)]
mod tests {
    #[test]
    fn examples_follow_the_actual_compiled_inventory_and_selection() {
        let global = super::Global {
            config_dir: Some(crate::settings::temp_dir("examples-inventory")),
            ..Default::default()
        };
        let text = "Usage: allowit rail use <RAIL>\n\nExamples:\n  allowit rail use __ALLOWIT_EXAMPLE_RAIL__\n\nOptions:\n  --help";
        let rendered = super::render(text, &global);
        match crate::routing::all().first() {
            Some(first) => {
                assert!(rendered.contains(&format!("allowit rail use {}", first.id)));
                let selected = crate::routing::all().last().unwrap();
                let global = super::Global {
                    rail: Some(selected.id.into()),
                    ..global
                };
                assert!(
                    super::render(text, &global)
                        .contains(&format!("allowit rail use {}", selected.id))
                );
            }
            None => {
                assert!(!rendered.contains("Examples:"));
                assert!(!rendered.contains("__ALLOWIT_EXAMPLE_RAIL__"));
            }
        }
    }
}
