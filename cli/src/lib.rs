//! The `allowit` command. Argument parsing, terminal output and rail adapters
//! live here; engine requests and service composition come from
//! `allowit-sdk-engine`, which has no terminal I/O.
pub use allowit_sdk::{error, wallet};
/// This CLI's own release version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) use allowit_sdk::{config, output, request};
mod commands;
mod ctx;
mod exit;
mod kinds;
mod owner;
mod rails;
mod receipt;
mod routing;
mod settings;
mod term;
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
    argv.iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--json")
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
    let mut out = Out::new(global.json, global.quiet, global.verbose, global.no_color);
    out.live = true;
    allowit_sdk::progress::set_sink(progress_sink(global.json, global.quiet, global.verbose));
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
    let result = routing::with_network(network, || commands::dispatch(command, &mut cx));
    finish(cx.out, result)
}

/// One long provider step as a stderr line, or `None` when the flags keep
/// stderr quiet. Text mode prints the step unless `-q` is set. With `--json`,
/// stderr holds JSON objects only, so a step prints only with `-v`, as one
/// `{"progress":{"message":…}}` object. Stdout never receives a step.
fn progress_line(json: bool, quiet: bool, verbose: bool, step: &str) -> Option<String> {
    match (json, quiet, verbose) {
        (_, true, _) | (true, _, false) => None,
        (true, false, true) => Some(json!({"progress": {"message": step}}).to_string()),
        (false, false, _) => Some(step.to_owned()),
    }
}

type ProgressSink = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// The sink that writes each provider step to stderr as it happens.
fn progress_sink(json: bool, quiet: bool, verbose: bool) -> Option<ProgressSink> {
    progress_line(json, quiet, verbose, "")?;
    Some(std::sync::Arc::new(move |step: &str| {
        if let Some(line) = progress_line(json, quiet, verbose, step) {
            let _ = writeln!(std::io::stderr(), "{}", redact(line));
        }
    }))
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

/// Writes an error to stderr. A typed server refusal keeps the server's
/// stable code: `api_code` in JSON, and before the message in text.
fn render_error(out: &mut Out, e: &Error) {
    if out.json {
        let mut value = json!({"error":{
            "code": Exit::from_code(e.code).name(),
            "exit": e.code,
            "message": e.message,
            "request_id": out.request_id,
        }});
        if let Some(code) = &e.api_code {
            value["error"]["api_code"] = code.as_str().into();
        }
        out.stderr += &format!("{value}\n");
    } else {
        let label = out.error_label();
        match &e.api_code {
            Some(code) => out.stderr += &format!("{label} {code}: {}\n", e.message),
            None => out.stderr += &format!("{label} {}\n", e.message),
        }
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
    fn a_typed_server_refusal_prints_its_code_and_keeps_its_exit_code() {
        use crate::error::Error;
        // (HTTP status, server code, exit code): typed refusals are final,
        // a pending transaction is uncertain and an unavailable network retries.
        for (status, code, exit) in [
            (400, "wallet_kind_unsupported", 4),
            (400, "wallet_kind_unknown", 4),
            (409, "owner_transaction_expired", 4),
            (409, "owner_transaction_settled", 4),
        ] {
            let error = Error::refused(status, code, "the server's reason");
            assert_eq!(error.code, exit, "{code}");
            let mut out = crate::term::Out::new(false, true, false, true);
            super::render_error(&mut out, &error);
            assert_eq!(
                out.stderr,
                format!("error: {code}: Allowit returned HTTP {status}: the server's reason\n")
            );
            let mut out = crate::term::Out::new(true, true, false, true);
            out.request_id = Some("deploy-0001".into());
            super::render_error(&mut out, &error);
            let value: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
            assert_eq!(
                value,
                serde_json::json!({"error": {
                    "code": "rejected", "exit": exit, "api_code": code,
                    "message": format!("Allowit returned HTTP {status}: the server's reason"),
                    "request_id": "deploy-0001",
                }})
            );
        }
        // An untyped error keeps the JSON shape without `api_code`.
        let mut out = crate::term::Out::new(true, true, false, true);
        super::render_error(&mut out, &Error::uncertain("no answer"));
        let value: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
        assert!(value["error"].get("api_code").is_none());
        assert_eq!(value["error"]["exit"], 5);
    }
    #[test]
    fn progress_goes_to_stderr_text_and_stays_out_of_json_unless_verbose() {
        // (json, quiet, verbose) -> the stderr line of one step.
        assert_eq!(
            super::progress_line(false, false, false, "Submitting").as_deref(),
            Some("Submitting")
        );
        assert_eq!(super::progress_line(false, true, false, "Submitting"), None);
        assert_eq!(super::progress_line(true, false, false, "Submitting"), None);
        assert_eq!(
            super::progress_line(true, false, true, "Submitting").as_deref(),
            Some(r#"{"progress":{"message":"Submitting"}}"#)
        );
        assert!(super::progress_sink(false, false, false).is_some());
        assert!(super::progress_sink(true, false, false).is_none());
        assert!(super::progress_sink(false, true, false).is_none());
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
