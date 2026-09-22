//! Reading the text inside a workflow: secret references and conditions.
//!
//! These are the places a contract is tempted to test with a substring, and
//! each substring test has a known way to pass while the rule it guards is
//! broken. A condition containing the main-branch guard can still make that
//! guard optional with `||`, and a secret can be read through any of three
//! spellings in any letter case. The readers here answer those questions by
//! shape instead, through two newtypes, [`Secret`] and [`Condition`], so a
//! secret's name and a condition's text cannot be passed where the other is
//! meant.

use serde_norway::Value;

/// A secret, named as the `secrets` context spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Secret(pub(crate) &'static str);

/// Which part of a node a secret search covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// The whole node.
    Whole,
    /// The node without one top-level key, which a narrower scope reports.
    Without(&'static str),
}

/// A key path within a node, such as `env.CS_ACCESS_TOKEN` or `steps[2]`.
#[derive(Debug, Clone, Default)]
struct KeyPath(String);

impl KeyPath {
    /// Return the path to a mapping entry under this one.
    fn child(&self, key: &Value) -> Self {
        let segment = key
            .as_str()
            .map_or_else(|| format!("{key:?}"), str::to_owned);
        if self.0.is_empty() {
            Self(segment)
        } else {
            Self(format!("{}.{segment}", self.0))
        }
    }

    /// Return the path to a sequence item under this one.
    fn item(&self, index: usize) -> Self {
        Self(format!("{}[{index}]", self.0))
    }
}

impl Secret {
    /// Return whether `text` reads this secret from the `secrets` context.
    ///
    /// GitHub resolves context properties case-insensitively and accepts both
    /// the dotted and the indexed spelling, so each is tried against a
    /// lower-cased, whitespace-free copy of the text. `toJSON(secrets)`
    /// serializes every secret at once, this one included, and counts as
    /// reading it. A match must end at a name boundary, or
    /// `secrets.cs_access_token_old` would read as the token.
    ///
    /// ```ignore
    /// let token = Secret("CS_ACCESS_TOKEN");
    /// assert!(token.is_read_by("${{ secrets.cs_access_token }}"));
    /// assert!(!token.is_read_by("${{ secrets.CS_ACCESS_TOKEN_OLD }}"));
    /// ```
    pub(crate) fn is_read_by(self, text: &str) -> bool {
        let compact: String = text
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let name = self.0.to_ascii_lowercase();
        let is_whole = |spelling: &String| {
            compact.match_indices(spelling.as_str()).any(|(at, _)| {
                compact
                    .get(at + spelling.len()..)
                    .and_then(|rest| rest.chars().next())
                    .is_none_or(|next| {
                        !(next.is_ascii_alphanumeric() || next == '_' || next == '-')
                    })
            })
        };
        compact.contains("tojson(secrets)")
            || [
                format!("secrets.{name}"),
                format!("secrets['{name}']"),
                format!("secrets[\"{name}\"]"),
            ]
            .iter()
            .any(is_whole)
    }

    /// Return the key path of every text value in `node` that reads this
    /// secret, within `scope`.
    ///
    /// ```ignore
    /// let step: Value = serde_norway::from_str("env: {A: '${{ secrets.T }}'}")?;
    /// assert_eq!(Secret("T").sites_in(&step, Scope::Whole), vec!["env.A"]);
    /// ```
    pub(crate) fn sites_in(self, node: &Value, scope: Scope) -> Vec<String> {
        match (node, scope) {
            (Value::Mapping(map), Scope::Without(skipped)) => map
                .iter()
                .filter(|(key, _)| key.as_str() != Some(skipped))
                .flat_map(|(key, value)| self.walk(value, &KeyPath::default().child(key)))
                .collect(),
            _ => self.walk(node, &KeyPath::default()),
        }
    }

    /// Return the path of every text value under `node` that reads this
    /// secret, each one below `path`.
    fn walk(self, node: &Value, path: &KeyPath) -> Vec<String> {
        match node {
            Value::Mapping(map) => map
                .iter()
                .flat_map(|(key, value)| self.walk(value, &path.child(key)))
                .collect(),
            Value::Sequence(items) => items
                .iter()
                .enumerate()
                .flat_map(|(index, item)| self.walk(item, &path.item(index)))
                .collect(),
            Value::Tagged(tagged) => self.walk(&tagged.value, path),
            Value::String(text) if self.is_read_by(text) => vec![path.0.clone()],
            _ => Vec::new(),
        }
    }
}

/// An `if` condition, as a workflow writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Condition<'a>(pub(crate) &'a str);

impl<'a> Condition<'a> {
    /// Return the condition's conjuncts, or the condition itself when it has
    /// an unquoted `||`.
    ///
    /// The condition is split on every `&&` outside a quoted string, and each
    /// conjunct has its whitespace normalized so that `a  ==  b` reads as
    /// `a == b`. An unquoted `||` is refused rather than split on: one
    /// disjunct makes every conjunct beside it optional, so
    /// `guard && ref == main || event == dispatch` contains the main-branch
    /// guard and enforces nothing.
    ///
    /// ```ignore
    /// let parts = Condition("${{ a && b == 'x || y' }}").conjuncts();
    /// assert_eq!(parts, Ok(vec!["a".into(), "b == 'x || y'".into()]));
    /// assert!(Condition("a || b").conjuncts().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the condition itself when it contains an unquoted `||`.
    pub(crate) fn conjuncts(self) -> Result<Vec<String>, String> {
        let mut parts = vec![String::new()];
        let mut is_quoted = false;
        let mut characters = self.body().chars().peekable();
        while let Some(character) = characters.next() {
            let doubled = !is_quoted && characters.peek() == Some(&character);
            match character {
                '\'' => is_quoted = !is_quoted,
                '|' if doubled => return Err(self.0.to_owned()),
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
        Ok(parts
            .iter()
            .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect())
    }

    /// Return the condition without its optional `${{ }}` wrapper.
    fn body(self) -> &'a str {
        let trimmed = self.0.trim();
        trimmed
            .strip_prefix("${{")
            .and_then(|inner| inner.strip_suffix("}}"))
            .unwrap_or(trimmed)
    }
}
