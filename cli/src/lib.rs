//! The `allowit` command. Argument parsing, terminal output and rail adapters
//! live here; engine requests and service composition come from
//! `allowit-sdk-engine`, which has no terminal I/O.
pub use allowit_sdk_engine::{VERSION, error, wallet};
pub(crate) use allowit_sdk_engine::{config, output, request, routing};
mod commands;
mod ctx;
mod exit;
mod owner;
mod settings;
mod term;
pub mod vendors;
use crate::{config::env_value, error::Error, exit::Exit, term::Out};
use serde_json::{Value, json};
use std::io::Write;

/// Every command path, hidden aliases included, for the help snapshot tests.
#[doc(hidden)]
pub fn command_paths() -> Vec<Vec<String>> {
    commands::paths()
}

/// Runs the command line (without the program name) from raw OS arguments.
/// An argument that is not UTF-8 is a usage error (exit 2).
pub fn run_os(args: Vec<std::ffi::OsString>) -> i32 {
    let text: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match args.iter().position(|a| a.to_str().is_none()) {
        Some(at) => {
            let message = format!("argument {} is not valid UTF-8", at + 1);
            usage_error(
                json_requested(&text),
                &message,
                &format!("error: {message}\n"),
            );
            Exit::Usage.code()
        }
        None => run(text),
    }
}

/// True when the arguments before `--` ask for JSON output.
fn json_requested(argv: &[String]) -> bool {
    let flags: Vec<&str> = argv
        .iter()
        .take_while(|a| a.as_str() != "--")
        .map(String::as_str)
        .collect();
    let output = |format: &str| {
        flags
            .windows(2)
            .any(|w| w[0] == "--output" && w[1] == format)
            || flags.contains(&format!("--output={format}").as_str())
    };
    flags.contains(&"--json") || output("json") || (term::no_dna() && !output("text"))
}

/// Writes a usage error to stderr: one JSON object, or the text.
fn usage_error(json: bool, first_line: &str, text: &str) {
    if json {
        let value = json!({"error":{"code":Exit::Usage.name(),"exit":Exit::Usage.code(),"message":first_line,"request_id":Value::Null}});
        let _ = writeln!(std::io::stderr(), "{value}");
    } else {
        let _ = std::io::stderr().write_all(text.as_bytes());
    }
}

/// Runs the command line (without the program name) and returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let mut argv = vec!["allowit".to_owned()];
    argv.extend(args);
    // Before parsing, so that parse errors honor them too.
    let json = json_requested(&argv);
    let no_color = argv
        .iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--no-color");
    match commands::parse(&argv, term::color_allowed(no_color, true)) {
        commands::Parsed::Print(text) => {
            let _ = std::io::stdout().write_all(text.as_bytes());
            0
        }
        commands::Parsed::Error(e) => {
            let code = Exit::Usage.code();
            let mut message = e.render().to_string();
            if e.kind() == clap::error::ErrorKind::InvalidSubcommand {
                let commands = commands::available_commands(&argv);
                if !commands.is_empty() {
                    message = format!(
                        "{}\n\nCommands: {}\n",
                        message.trim_end(),
                        commands.join(", ")
                    );
                }
            }
            let text = strip_ansi(&message);
            let first = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("usage error")
                .trim_start_matches("error: ");
            usage_error(json, first, &message);
            code
        }
        commands::Parsed::Run(cli) => execute(*cli),
    }
}

fn strip_ansi(text: &str) -> String {
    regex::Regex::new("\x1b\\[[0-9;]*m")
        .map(|re| re.replace_all(text, "").into_owned())
        .unwrap_or_else(|_| text.to_owned())
}

fn execute(cli: commands::Cli) -> i32 {
    let global = cli.global;
    let mut out = Out::new(
        global.json_output(),
        global.quiet,
        global.verbose,
        global.no_color,
    );
    out.live = true;
    let mut cx = ctx::Ctx::new(global, out);
    let Some(command) = cli.command else {
        return 0;
    };
    // Commands that read no rail never resolve one, so a broken rail setting
    // cannot change their output.
    let network = if commands::is_local(&command) {
        None
    } else {
        cx.rail_opt().ok().flatten().map(|r| r.id.to_owned())
    };
    if let Some(n) = &network
        && cx.global.verbose
        && let Ok(dir) = cx.dir()
    {
        cx.out.detail(format!("rail  {n}"));
        cx.out.detail(format!("config  {}", dir.display()));
    }
    let settings = match &network {
        Some(n) => {
            match routing::info(n)
                .map(vendors::rail)
                .and_then(|v| v.settings(&mut cx))
            {
                Ok(settings) => settings,
                Err(e) => return finish(cx.out, Err(e)),
            }
        }
        _ => vec![],
    };
    let result = routing::with_network(network, || {
        config::with_settings(&settings, || commands::dispatch(command, &mut cx))
    });
    finish(cx.out, result)
}

fn finish(mut out: Out, result: error::Result<i32>) -> i32 {
    let code = match result {
        Ok(code) => code,
        Err(e) => {
            render_error(&mut out, &e);
            e.code
        }
    };
    let stdout = std::mem::take(&mut out.stdout);
    let stdout = if out.raw {
        stdout
    } else {
        redact(safe_output(stdout))
    };
    let stdout = out.rendered_stdout(stdout);
    let _ = std::io::stdout().write_all(stdout.as_bytes());
    let _ = std::io::stderr().write_all(redact(std::mem::take(&mut out.stderr)).as_bytes());
    code
}

fn render_error(out: &mut Out, e: &Error) {
    if out.json {
        let value = json!({"error":{
            "code": Exit::from_code(e.code).name(),
            "exit": e.code,
            "message": e.message,
            "request_id": out.request_id,
        }});
        out.stderr += &format!("{value}\n");
    } else {
        let label = out.error_label();
        out.stderr += &format!("{label} {}\n", e.message);
    }
}

/// True for characters that can change the terminal or reorder text:
/// C0 controls other than newline and tab, DEL, C1 controls and the bidi
/// embedding, override and isolate characters.
fn display_control(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || ('\u{202a}'..='\u{202e}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
}

/// Escapes display controls in stdout. JSON keeps its value: a control
/// inside a string becomes a `\uXXXX` escape. Text shows each control as
/// `\u{XX}`.
fn safe_output(stdout: String) -> String {
    let json = serde_json::from_str::<Value>(&stdout).is_ok();
    if !stdout.chars().any(display_control) {
        return stdout;
    }
    stdout
        .chars()
        .map(|c| {
            if !display_control(c) {
                c.to_string()
            } else if json {
                format!("\\u{:04x}", c as u32)
            } else {
                format!("\\u{{{:x}}}", c as u32)
            }
        })
        .collect()
}

/// Removes the engine tokens from any output.
pub(crate) fn redact(s: String) -> String {
    output::clean(
        output::clean(s, &env_value("ALLOWIT_TOKEN")),
        &env_value("ALLOWIT_ACCOUNT_TOKEN"),
    )
}

/// Writes pending stderr now, before a request can leave the process.
pub(crate) fn flush_stderr(stderr: &mut String) {
    let _ = std::io::stderr().write_all(redact(std::mem::take(stderr)).as_bytes());
}

#[cfg(test)]
mod display_tests {
    #[test]
    fn display_controls_are_escaped_in_json_and_text() {
        let original = serde_json::json!({"prompt":"source\u{202e} text\u{0085}\u{1b}]52;x"});
        let encoded = super::safe_output(serde_json::to_string_pretty(&original).unwrap());
        assert!(!encoded.contains('\u{202e}'));
        assert!(!encoded.contains('\u{0085}'));
        assert!(!encoded.contains('\u{1b}'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
            original
        );
        let text = "name  agent\u{1b}]52;c;aGk=\u{7}\nnote  a\u{202e}b\tc\r\n".to_owned();
        let shown = super::safe_output(text);
        assert_eq!(
            shown,
            "name  agent\\u{1b}]52;c;aGk=\\u{7}\nnote  a\\u{202e}b\tc\\u{d}\n"
        );
        assert_eq!(super::safe_output("plain\n".into()), "plain\n");
    }
    #[test]
    fn an_error_keeps_the_exit_code_that_its_command_set() {
        // Contract error numbers are mapped only inside their rail, so a
        // message that names one keeps its code here.
        for message in [
            "simulation failed: Error(Contract, #5)",
            "send failed: custom program error: 0x68",
        ] {
            let out = crate::term::Out::new(true, true, false, true);
            let error = crate::error::Error::config(message);
            assert_eq!(super::finish(out, Err(error)), 3, "{message}");
        }
    }
}
