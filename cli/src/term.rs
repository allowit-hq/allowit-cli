//! Terminal output. Data goes to stdout, including an owner signing link as
//! the `signingUrl` field; progress, hints and prompts go to stderr. Text is
//! the default, also when piped; `--json` (or `--output json`) selects JSON,
//! and policy source prints as a `source` field. Policy source and JSON documents are
//! syntax-highlighted only when stdout is a terminal, and never with `--json`,
//! `NO_COLOR`, `TERM=dumb` or `--no-color`. `CLICOLOR_FORCE` forces color.
use serde_json::Value;
use std::io::IsTerminal;

/// What stdout holds, for highlighting at flush time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Text,
    Json,
    Rust,
}

/// Buffered command output, written once when the command ends.
pub(crate) struct Out {
    pub stdout: String,
    pub stderr: String,
    pub json: bool,
    /// `NO_DNA`: compact JSON, no prompts.
    pub compact: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub color_out: bool,
    pub color_err: bool,
    pub kind: Kind,
    /// Print stdout unchanged: no redaction and no display escaping. The local
    /// policy source commands use it.
    pub raw: bool,
    /// The request this command acts on, for JSON errors.
    pub request_id: Option<String>,
    /// Write progress, with any pending stderr, at once instead of when the
    /// command ends, so an interrupted command keeps its request ID.
    pub live: bool,
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

/// True when color may be written to a stream that is a terminal.
pub(crate) fn color_allowed(no_color_flag: bool, terminal: bool) -> bool {
    if no_color_flag || env_set("NO_COLOR") || std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        return false;
    }
    env_set("CLICOLOR_FORCE") || terminal
}

/// `NO_DNA`: compact JSON output and errors, and no prompts.
pub(crate) fn no_dna() -> bool {
    env_set("NO_DNA")
}

impl Out {
    pub(crate) fn new(json: bool, quiet: bool, verbose: bool, no_color: bool) -> Self {
        let compact = no_dna();
        Self {
            stdout: String::new(),
            stderr: String::new(),
            json,
            compact,
            quiet,
            verbose,
            color_out: !json
                && !compact
                && color_allowed(no_color, std::io::stdout().is_terminal()),
            color_err: !compact && color_allowed(no_color, std::io::stderr().is_terminal()),
            kind: Kind::Text,
            raw: false,
            request_id: None,
            live: false,
        }
    }
    /// One line of text data.
    pub(crate) fn line(&mut self, text: impl AsRef<str>) {
        self.stdout.push_str(text.as_ref());
        self.stdout.push('\n');
    }
    /// A structured result: JSON in JSON mode, [`Out::text`] otherwise.
    pub(crate) fn value(&mut self, value: &Value) {
        if self.json {
            self.json_value(value);
        } else {
            self.text(value);
        }
    }
    /// A structured result as text; see [`text`].
    pub(crate) fn text(&mut self, value: &Value) {
        self.stdout.push_str(&text(value));
    }
    /// A JSON document, highlighted on a terminal in text mode.
    pub(crate) fn json_value(&mut self, value: &Value) {
        let text = if self.compact {
            serde_json::to_string(value)
        } else {
            serde_json::to_string_pretty(value)
        }
        .unwrap_or_default();
        self.stdout.push_str(&text);
        self.stdout.push('\n');
        if self.kind == Kind::Text {
            self.kind = Kind::Json;
        }
    }
    /// Rust policy source, highlighted on a terminal. With `--json` it is the
    /// `source` field of one JSON object, so stdout stays one JSON document.
    pub(crate) fn source(&mut self, rust: &str) {
        if self.json {
            self.json_value(&serde_json::json!({ "source": rust }));
            return;
        }
        self.stdout.push_str(rust);
        if !rust.ends_with('\n') {
            self.stdout.push('\n');
        }
        self.kind = Kind::Rust;
    }
    /// A table of rows; the first row is the header. Text mode only.
    pub(crate) fn table(&mut self, rows: &[Vec<String>]) {
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|c| {
                rows.iter()
                    .filter_map(|r| r.get(c))
                    .map(|s| s.chars().count())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        for row in rows {
            let mut line = String::new();
            for (c, cell) in row.iter().enumerate() {
                if c + 1 == row.len() {
                    line.push_str(cell);
                } else {
                    line.push_str(&format!("{cell:width$}  ", width = widths[c]));
                }
            }
            self.line(line.trim_end());
        }
    }
    /// Progress on stderr. `-q` drops it.
    pub(crate) fn progress(&mut self, text: impl AsRef<str>) {
        if !self.quiet {
            self.stderr.push_str(text.as_ref());
            self.stderr.push('\n');
            if self.live {
                crate::flush_stderr(&mut self.stderr);
            }
        }
    }
    /// A generated ID on stderr, which a retry needs. `-q` keeps it.
    pub(crate) fn generated_id(&mut self, id: &str) {
        let quiet = std::mem::replace(&mut self.quiet, false);
        self.progress(id);
        self.quiet = quiet;
    }
    /// A problem that leaves the result unchanged, on stderr. `-q` keeps it.
    pub(crate) fn warning(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        if self.json {
            let value = serde_json::json!({"warning": {"message": text}});
            self.stderr.push_str(&format!("{value}\n"));
        } else if self.color_err {
            self.stderr
                .push_str(&format!("\x1b[1;33mwarning:\x1b[0m {text}\n"));
        } else {
            self.stderr.push_str(&format!("warning: {text}\n"));
        }
    }
    /// Extra diagnostics on stderr with `-v`.
    pub(crate) fn detail(&mut self, text: impl AsRef<str>) {
        if self.verbose {
            self.stderr.push_str(text.as_ref());
            self.stderr.push('\n');
        }
    }
    /// The final stdout text, highlighted when allowed.
    pub(crate) fn rendered_stdout(&self, stdout: String) -> String {
        if !self.color_out || stdout.is_empty() {
            return stdout;
        }
        match self.kind {
            Kind::Rust => highlight(&stdout, "rs"),
            Kind::Json => highlight(&stdout, "json"),
            Kind::Text if serde_json::from_str::<Value>(&stdout).is_ok() => {
                highlight(&stdout, "json")
            }
            Kind::Text => stdout,
        }
    }
    /// An error label for stderr, red on a terminal.
    pub(crate) fn error_label(&self) -> &'static str {
        if self.color_err {
            "\x1b[1;31merror:\x1b[0m"
        } else {
            "error:"
        }
    }
}

/// A structured result as text: one `key  value` line per scalar, with
/// dotted paths for nested fields (`config.rail`, `steps.0.name`). Lists of
/// scalars join with commas; a multi-line string follows its key, indented.
/// Null and empty values print as `-`.
pub(crate) fn text(value: &Value) -> String {
    let mut rows = vec![];
    flatten(String::new(), value, &mut rows);
    let width = rows
        .iter()
        .filter(|(_, v)| !v.contains('\n'))
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push('\n');
    };
    for (key, text) in rows {
        if key.is_empty() {
            line(&text);
        } else if text.contains('\n') {
            line(&key);
            for l in text.lines() {
                line(format!("  {l}").trim_end());
            }
        } else {
            line(&format!("{key:width$}  {text}"));
        }
    }
    out
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) if s.is_empty() => "-".into(),
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

/// Collects `(path, text)` rows for [`Out::text`].
fn flatten(prefix: String, value: &Value, rows: &mut Vec<(String, String)>) {
    let join = |key: &str| {
        if prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{prefix}.{key}")
        }
    };
    match value {
        Value::Object(map) if map.is_empty() => rows.push((prefix, "-".into())),
        Value::Object(map) => {
            for (k, v) in map {
                flatten(join(k), v, rows);
            }
        }
        Value::Array(items) if items.is_empty() => rows.push((prefix, "-".into())),
        Value::Array(items) if items.iter().all(|v| !v.is_object() && !v.is_array()) => {
            let text = items.iter().map(scalar).collect::<Vec<_>>().join(", ");
            rows.push((prefix, text));
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                flatten(join(&i.to_string()), v, rows);
            }
        }
        other => rows.push((prefix, scalar(other))),
    }
}

/// Highlights text with the syntax for a file extension, as 24-bit ANSI color.
pub(crate) fn highlight(text: &str, extension: &str) -> String {
    use syntect::{
        easy::HighlightLines,
        highlighting::ThemeSet,
        parsing::SyntaxSet,
        util::{LinesWithEndings, as_24_bit_terminal_escaped},
    };
    let syntaxes = SyntaxSet::load_defaults_newlines();
    let Some(syntax) = syntaxes.find_syntax_by_extension(extension) else {
        return text.to_owned();
    };
    let themes = ThemeSet::load_defaults();
    let Some(theme) = themes.themes.get("base16-ocean.dark") else {
        return text.to_owned();
    };
    let mut lines = HighlightLines::new(syntax, theme);
    let mut out = String::with_capacity(text.len() * 2);
    for line in LinesWithEndings::from(text) {
        match lines.highlight_line(line, &syntaxes) {
            Ok(ranges) => out.push_str(&as_24_bit_terminal_escaped(&ranges, false)),
            Err(_) => return text.to_owned(),
        }
    }
    out.push_str("\x1b[0m");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn plain(json: bool) -> Out {
        let mut out = Out::new(json, false, false, true);
        out.compact = false;
        out
    }
    #[test]
    fn values_print_as_aligned_text_lines_and_json_only_in_json_mode() {
        let mut out = plain(false);
        out.value(&json!({"name":"agent","address":"GABC"}));
        // Key order follows serde_json's map order; alignment does not.
        let mut lines: Vec<&str> = out.stdout.lines().collect();
        lines.sort();
        assert_eq!(lines, ["address  GABC", "name     agent"]);
        let mut out = plain(true);
        out.value(&json!({"name":"agent"}));
        assert_eq!(out.stdout, "{\n  \"name\": \"agent\"\n}\n");
        let mut out = plain(false);
        out.value(&json!({
            "rail": "stellar:testnet",
            "config": {"rail": "stellar:testnet", "rails": {}},
            "keys": ["a", "b"],
            "steps": [{"name": "fund"}],
            "none": null,
            "empty": "",
            "source": "fn a() {}\nfn b() {}",
        }));
        let text = out.stdout.clone();
        assert!(serde_json::from_str::<Value>(&text).is_err());
        assert_eq!(out.kind, Kind::Text);
        for (key, value) in [
            ("rail", "stellar:testnet"),
            ("config.rail", "stellar:testnet"),
            ("config.rails", "-"),
            ("keys", "a, b"),
            ("steps.0.name", "fund"),
            ("none", "-"),
            ("empty", "-"),
        ] {
            let line = format!("{key:12}  {value}\n");
            assert!(text.contains(&line), "{line:?} in\n{text}");
        }
        assert!(text.contains("source\n  fn a() {}\n  fn b() {}\n"));
        let mut out = plain(false);
        let long = "credentials.ALLOWIT_ACCOUNT_TOKEN";
        out.value(&json!({"credentials": {"ALLOWIT_ACCOUNT_TOKEN": "set"}, "rail": "x"}));
        assert!(
            out.stdout.contains(&format!("{long}  set\n")),
            "{}",
            out.stdout
        );
        assert!(
            out.stdout
                .contains(&format!("{:w$}  x\n", "rail", w = long.len()))
        );
        let mut out = plain(false);
        out.value(&json!([{"a": 1}, {"a": 2}]));
        assert_eq!(out.stdout, "0.a  1\n1.a  2\n");
        let mut out = plain(false);
        out.value(&json!("plain"));
        assert_eq!(out.stdout, "plain\n");
    }
    #[test]
    fn highlighting_adds_color_and_keeps_the_text() {
        let source = "fn main() { let x = 1; }\n";
        let colored = highlight(source, "rs");
        assert!(colored.contains("\x1b["));
        let stripped = regex::Regex::new("\x1b\\[[0-9;]*m")
            .unwrap()
            .replace_all(&colored, "");
        assert_eq!(stripped, source);
        let mut out = plain(false);
        out.source(source);
        assert_eq!(out.rendered_stdout(out.stdout.clone()), source);
        out.color_out = true;
        assert!(out.rendered_stdout(out.stdout.clone()).contains("\x1b["));
    }
    #[test]
    fn live_progress_leaves_at_once() {
        let mut out = Out::new(false, false, false, true);
        out.stderr.push_str("pending\n");
        out.live = true;
        out.progress("request-0001");
        assert!(out.stderr.is_empty());
    }
    #[test]
    fn quiet_drops_progress_but_not_data() {
        let mut out = Out::new(false, true, false, true);
        out.progress("working");
        out.line("data");
        assert!(out.stderr.is_empty());
        assert_eq!(out.stdout, "data\n");
    }
}
