//! Every typed error code that this CLI names has an entry in the error catalogue
//! (`allowit_sdk::errors`). The command line prints the catalogue's cause, next step and help
//! page for a code, so a code without an entry would print as raw text only.
//!
//! The test reads the CLI's source. A code is: the code argument of a refusal that the CLI makes
//! (`refused(status, "code", …)`, `refused("code", …)`, `api_code = Some("code".into())`), a code
//! that the CLI compares a refusal with (`Some("code")`, `["code"] == "code"`), and a string
//! constant with a code-like value. A constant that is no error code is in `NOT_CODES`.
use allowit_sdk::errors;
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;

/// Code-like constants and compared words of the CLI that are not error codes.
const NOT_CODES: &[(&str, &str)] = &[
    ("awaiting_agent_key", "a state of a policy"),
    ("same_request_id", "the retry word of a refusal's details"),
    ("free_quota", "how a catalog entry gives access"),
    ("proof_of_work", "how a catalog entry gives access"),
];

fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            // The unit tests of a file use invented codes for refusals of a newer server.
            let production = text
                .find("#[cfg(test)]\nmod ")
                .map_or(text.as_str(), |end| &text[..end])
                .to_owned();
            out.push((path.display().to_string(), production));
        }
    }
}

#[test]
fn every_typed_code_that_the_cli_names_has_a_catalogue_entry() {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 30);
    let code = r#""([a-z][a-z0-9]*(?:_[a-z0-9]+)+)""#;
    let forms = [
        // A refusal that the CLI makes, with or without an HTTP status.
        format!(r"refused\(\s*(?:[A-Za-z0-9_.]+\s*,\s*)?{code}"),
        format!(r"api_code\s*=\s*Some\(\s*{code}"),
        // A code that the CLI compares a refusal with.
        format!(r"Some\(\s*{code}\s*(?:\||\))"),
        format!(r#"\["code"\]\s*==\s*{code}"#),
        // A string constant with a code-like value.
        format!(r"const [A-Z][A-Z0-9_]*: &str =\s*{code}"),
    ]
    .map(|form| Regex::new(&form).unwrap());
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for (file, text) in &files {
        for form in &forms {
            for capture in form.captures_iter(text) {
                found
                    .entry(capture[1].to_owned())
                    .or_insert_with(|| file.clone());
            }
        }
    }
    // The codes that the CLI is known to name: the scan found them.
    for expected in [
        "policy_unknown",
        "not_policy_agent",
        "request_proof_refused",
        "server_release_differs",
        "service_unreachable",
        "server_key_differs",
        "policy_not_active",
        "wallet_differs",
        "wallet_unverified",
        "credits_exhausted",
        "evaluation_running",
        "credits_unavailable",
        "challenge_already_paid",
    ] {
        assert!(
            found.contains_key(expected),
            "{expected} was not found in the source"
        );
    }
    let not_codes: Vec<&str> = NOT_CODES.iter().map(|(word, _)| *word).collect();
    let missing: Vec<String> = found
        .iter()
        .filter(|(code, _)| !not_codes.contains(&code.as_str()) && errors::entry(code).is_none())
        .map(|(code, file)| format!("{code} ({file})"))
        .collect();
    assert!(
        missing.is_empty(),
        "these typed codes have no entry in the error catalogue of allowit-core \
         (typescript/error-catalogue.json); add the entry there, or list a word that is no error \
         code in NOT_CODES:\n{}",
        missing.join("\n")
    );
    for (word, what) in NOT_CODES {
        assert!(
            errors::entry(word).is_none(),
            "{word} is listed as {what} and has an entry"
        );
    }
}
