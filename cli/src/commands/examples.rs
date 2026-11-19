//! Help examples name the effective local selection, else the placeholder `RAIL`. No example
//! takes a rail by build order: every rail is chosen by its id.
use super::Global;
use crate::{routing, settings::Settings};
use clap::{CommandFactory, FromArgMatches};

const RAIL_MARKER: &str = "__ALLOWIT_EXAMPLE_RAIL__";
/// The rail of an example when none is selected: the value name of `allowit rail use`.
const RAIL_PLACEHOLDER: &str = "RAIL";

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

pub(super) fn render(text: &str, global: &Global) -> String {
    text.replace(RAIL_MARKER, selected(global).unwrap_or(RAIL_PLACEHOLDER))
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
    fn examples_name_the_selected_rail_or_a_placeholder() {
        let global = super::Global {
            config_dir: Some(crate::settings::temp_dir("examples-inventory")),
            ..Default::default()
        };
        let text = "Usage: allowit rail use <RAIL>\n\nExamples:\n  allowit rail use __ALLOWIT_EXAMPLE_RAIL__\n\nOptions:\n  --help\n";
        // Without a selection no registered rail stands in, whatever the build order.
        let rendered = super::render(text, &global);
        assert!(
            rendered.contains("\n  allowit rail use RAIL\n"),
            "{rendered}"
        );
        assert!(rendered.ends_with('\n'));
        for rail in crate::routing::all() {
            assert!(!rendered.contains(rail.id), "{rendered}");
            let global = super::Global {
                rail: Some(rail.id.into()),
                ..global.clone()
            };
            assert!(
                super::render(text, &global).contains(&format!("allowit rail use {}\n", rail.id))
            );
        }
    }
}
