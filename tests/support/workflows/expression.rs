//! Reading the text inside a workflow: secret references, conditions and
//! reusable-workflow calls.
//!
//! These are the places a contract is tempted to test with a substring, and
//! each substring test has a known way to pass while the rule it guards is
//! broken. A condition containing the main-branch guard can still make that
//! guard optional with `||`; a secret can be read through any of three
//! spellings in any letter case; a local call can be written with or without
//! `./`. The readers here answer those questions by shape instead.

use serde_norway::Value;

/// The directory, relative to the repository root, that holds its workflows.
const WORKFLOW_DIRECTORY: &str = ".github/workflows/";

/// Return whether `text` reads `secret` from the `secrets` context.
///
/// GitHub resolves context properties case-insensitively and accepts both the
/// dotted and the indexed spelling, so each is tried against a lower-cased,
/// whitespace-free copy of the text. `toJSON(secrets)` serializes every
/// secret at once, the named one included, and counts as reading it.
///
/// ```ignore
/// assert!(references_secret("${{ secrets.cs_access_token }}", "CS_ACCESS_TOKEN"));
/// assert!(!references_secret("${{ secrets.CS_ACCESS_TOKEN_OLD }}", "CS_ACCESS_TOKEN"));
/// ```
pub(crate) fn references_secret(text: &str, secret: &str) -> bool {
    let compact: String = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let name = secret.to_ascii_lowercase();
    let spellings = [
        format!("secrets.{name}"),
        format!("secrets['{name}']"),
        format!("secrets[\"{name}\"]"),
    ];
    compact.contains("tojson(secrets)")
        || spellings
            .iter()
            .any(|spelling| names_whole(&compact, spelling))
}

/// Return whether `needle` occurs in `text` not followed by a name character.
///
/// Without the boundary, `secrets.cs_access_token_old` would read as the
/// token, and a contract would condemn a workflow for a secret it never uses.
fn names_whole(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(at, _)| {
        text.get(at + needle.len()..)
            .and_then(|rest| rest.chars().next())
            .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '_' || next == '-'))
    })
}

/// Return the key path of every text value under `node` that reads `secret`.
///
/// `skip` names one top-level key to leave out, so a workflow can report its
/// own sites without repeating every one its jobs report.
pub(crate) fn secret_sites(node: &Value, secret: &str, skip: Option<&str>) -> Vec<String> {
    match node {
        Value::Mapping(map) => map
            .iter()
            .filter(|(key, _)| skip.is_none_or(|skipped| key.as_str() != Some(skipped)))
            .flat_map(|(key, value)| {
                let segment = key
                    .as_str()
                    .map_or_else(|| format!("{key:?}"), str::to_owned);
                prefixed(&segment, secret_sites(value, secret, None))
            })
            .collect(),
        Value::Sequence(items) => items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                prefixed(&format!("[{index}]"), secret_sites(item, secret, None))
            })
            .collect(),
        Value::Tagged(tagged) => secret_sites(&tagged.value, secret, skip),
        Value::String(text) if references_secret(text, secret) => vec![String::new()],
        _ => Vec::new(),
    }
}

/// Return each path in `sites` with `segment` in front of it.
fn prefixed(segment: &str, sites: Vec<String>) -> Vec<String> {
    sites
        .into_iter()
        .map(|site| match site.as_str() {
            "" => segment.to_owned(),
            rest if rest.starts_with('[') => format!("{segment}{rest}"),
            rest => format!("{segment}.{rest}"),
        })
        .collect()
}

/// Return the conjuncts of an `if` condition, or why it has none.
///
/// The condition is split on every `&&` outside a quoted string, and each
/// conjunct has its whitespace normalized so that `a  ==  b` reads as
/// `a == b`. An unquoted `||` is refused rather than split on: one
/// disjunct makes every conjunct beside it optional, so
/// `guard && ref == main || event == dispatch` contains the main-branch guard
/// and enforces nothing.
///
/// ```ignore
/// assert_eq!(conjuncts("${{ a && b == 'x || y' }}"), Ok(vec!["a".into(), "b == 'x || y'".into()]));
/// assert!(conjuncts("a || b").is_err());
/// ```
///
/// # Errors
///
/// Returns the condition itself when it contains an unquoted `||`.
pub(crate) fn conjuncts(condition: &str) -> Result<Vec<String>, String> {
    let body = unwrap_expression(condition);
    let mut parts = vec![String::new()];
    let mut is_quoted = false;
    let mut characters = body.chars().peekable();
    while let Some(character) = characters.next() {
        let doubled = !is_quoted && characters.peek() == Some(&character);
        match character {
            '\'' => is_quoted = !is_quoted,
            '|' if doubled => return Err(condition.to_owned()),
            '&' if doubled => {
                characters.next();
                parts.push(String::new());
                continue;
            }
            _ => {}
        }
        if let Some(part) = parts.last_mut() {
            part.push(character);
        }
    }
    Ok(parts.iter().map(|part| normalized(part)).collect())
}

/// Return a condition without its optional `${{ }}` wrapper.
fn unwrap_expression(condition: &str) -> &str {
    let trimmed = condition.trim();
    trimmed
        .strip_prefix("${{")
        .and_then(|inner| inner.strip_suffix("}}"))
        .unwrap_or(trimmed)
}

/// Return `text` with each run of whitespace collapsed to one space.
fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Return the workflow file a job-level `uses:` names in this repository.
///
/// A local call is recognized by shape rather than by a list of prefixes:
/// strip a leading `./`, then ask whether the rest is a path under the
/// workflow directory. A call into another repository starts with its owner
/// and so never matches.
///
/// ```ignore
/// assert_eq!(local_call_target("./.github/workflows/probe.yml"), Some("probe.yml"));
/// assert_eq!(local_call_target("leynos/shared-actions/.github/workflows/x.yml@abc"), None);
/// ```
pub(crate) fn local_call_target(uses: &str) -> Option<&str> {
    uses.strip_prefix("./")
        .unwrap_or(uses)
        .strip_prefix(WORKFLOW_DIRECTORY)
}
