//! Shell tokenizing for the suite-once contract: splits a command into
//! segments of words with the shell's quote state, so a separator inside
//! quotes is literal and an unquoted comment is dropped.
//!
//! Kept apart from `suite_shell.rs` so each file stays under the repository's
//! 400-line limit. Only that module uses it.

use std::str::Chars;

/// One word of a command, with its quotes and escapes removed.
pub(super) struct Word(pub(super) String);

/// The words of one shell segment: a command between separators.
pub(super) struct Segment(Vec<Word>);

impl Segment {
    /// Returns the segment's words.
    pub(super) fn words(&self) -> &[Word] {
        &self.0
    }
}

/// Splits a command into segments of words with the shell's quote state.
#[derive(Default)]
pub(super) struct Tokenizer {
    /// The segments read so far.
    segments: Vec<Segment>,
    /// The words of the segment being read.
    words: Vec<Word>,
    /// The word being read, if one has begun.
    word: Option<String>,
    /// The quote character that is open, if any.
    quote: Option<char>,
}

impl Tokenizer {
    /// Splits `command` at `;`, `|`, `&`, `(`, `)` and newlines outside
    /// quotes, spaced or not. An unquoted `#` at the start of a word begins a comment that
    /// runs to the end of the line, and a quote left open runs to the end
    /// of the command, as it does in the shell.
    pub(super) fn read(mut self, command: &str) -> Vec<Segment> {
        let mut chars = command.chars();
        while let Some(c) = chars.next() {
            self.step(c, &mut chars);
        }
        self.end_segment();
        self.segments
    }

    /// Reads one character, consuming more of `rest` for an escape or a
    /// comment.
    fn step(&mut self, c: char, rest: &mut Chars<'_>) {
        match (self.quote, c) {
            (Some('"'), '\\') => self.escape_in_double_quotes(rest),
            (Some(open), _) if c == open => self.quote = None,
            (None, '\'' | '"') => {
                self.quote = Some(c);
                self.word.get_or_insert_with(String::new);
            }
            (None, '#') if self.word.is_none() => {
                rest.find(|next| *next == '\n');
                self.end_segment();
            }
            (None, '\\') => self.escape(rest.next()),
            (None, ';' | '|' | '&' | '\n' | '(' | ')') => self.end_segment(),
            (None, _) if c.is_whitespace() => self.end_word(),
            _ => self.push(c),
        }
    }

    /// Reads the character after a backslash: a line continuation is
    /// removed without ending the word, as the shell does, and anything else
    /// is literal.
    fn escape(&mut self, escaped: Option<char>) {
        match escaped {
            Some('\n') | None => {}
            Some(literal) => self.push(literal),
        }
    }

    /// Reads the character after a backslash inside double quotes: only `"`,
    /// `\`, `$` and a backtick are escaped, a newline continues the line, and
    /// anything else keeps its backslash.
    fn escape_in_double_quotes(&mut self, rest: &mut Chars<'_>) {
        match rest.next() {
            Some(escaped @ ('"' | '\\' | '$' | '`')) => self.push(escaped),
            Some('\n') | None => {}
            Some(other) => {
                self.push('\\');
                self.push(other);
            }
        }
    }

    /// Adds a character to the current word, starting one if needed.
    fn push(&mut self, c: char) {
        self.word.get_or_insert_with(String::new).push(c);
    }

    /// Ends the current word, if one has begun.
    fn end_word(&mut self) {
        self.words.extend(self.word.take().map(Word));
    }

    /// Ends the current word and segment.
    fn end_segment(&mut self) {
        self.end_word();
        self.segments.push(Segment(std::mem::take(&mut self.words)));
    }
}
