//! A lenient reading of an answer that fails the strict one, so a page whose
//! JSON a model broke still keeps the translations it did give. It runs only
//! after `turn::answers` refuses the answer, never in place of it.
//!
//! Fork divergence: upstream fails the page on any answer the strict parse
//! refuses. The parser is the one Koharu's translator used before the
//! pipeline's rewrite, kept with the fork's fixes for what models were seen to
//! write: a key whose closing quote swallowed the colon, unescaped quotes
//! inside a value, and two strings that lost the comma between them.

use std::fmt::Write as _;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Number, Value};

const MAX_DEPTH: usize = 128;
/// How many `{` the answer is tried from, the first being the outermost.
const MAX_ROOT_CANDIDATES: usize = 64;
/// How many characters of an answer an error quotes.
const SNIPPET: usize = 2_000;

/// What the answer holds for each of `keys`: a text that isn't blank, or
/// `None` where it's missing or corrupt. Fails only when no key has one.
pub(super) fn answers(answer: &str, keys: &[String]) -> Result<Vec<Option<String>>> {
    let mut first_error = None;
    for (position, _) in answer
        .char_indices()
        .filter(|&(_, character)| character == '{')
        .take(MAX_ROOT_CANDIDATES)
    {
        let found = match Parser::new(&answer[position..]).parse() {
            Ok(Value::Object(object)) => texts(&object, keys),
            Ok(_) => continue,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        if found.iter().any(Option::is_some) {
            return Ok(found);
        }
    }
    Err(first_error.unwrap_or_else(|| anyhow!("the answer has none of the page's texts")))
}

/// Each of `keys`' text in `object`. A key matches trimmed, and by its number
/// when both are numbers, as a model may drop the padding.
fn texts(object: &Map<String, Value>, keys: &[String]) -> Vec<Option<String>> {
    let normal = |key: &str| {
        let key = key.trim();
        match key.chars().all(|character| character.is_ascii_digit()) && !key.is_empty() {
            true => key.trim_start_matches('0').to_owned(),
            false => key.to_owned(),
        }
    };
    keys.iter()
        .map(|key| {
            let key = normal(key);
            object
                .iter()
                .find(|(found, _)| normal(found) == key)
                .and_then(|(_, value)| value.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        })
        .collect()
}

/// `answer` on one line, its control characters escaped, cut at `SNIPPET`
/// characters, for an error's context.
pub(super) fn snippet(answer: &str) -> String {
    let trimmed = answer.trim();
    let (visible, elided) = match trimmed.char_indices().nth(SNIPPET) {
        Some((end, _)) => (&trimmed[..end], true),
        None => (trimmed, false),
    };
    let mut output = String::with_capacity(visible.len());
    for character in visible.chars() {
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control.is_control() => {
                let _ = write!(output, "\\u{{{:04x}}}", control as u32);
            }
            character => output.push(character),
        }
    }
    if elided {
        let _ = write!(output, "… ({} bytes total)", trimmed.len());
    }
    output
}

struct Parser<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }

    fn parse(mut self) -> Result<Value> {
        self.skip_ignored();
        self.parse_value(0)
    }

    fn parse_value(&mut self, depth: usize) -> Result<Value> {
        if depth >= MAX_DEPTH {
            bail!("JSON nesting exceeds {MAX_DEPTH} levels");
        }
        self.skip_ignored();
        match self.peek() {
            Some('{') => self.parse_object(depth + 1),
            Some('[') => self.parse_array(depth + 1),
            Some(character) if quote_end(character).is_some() => {
                self.parse_string(true).map(Value::String)
            }
            Some(character) if character.is_ascii_digit() || matches!(character, '-' | '+') => {
                Ok(self.parse_number_or_string())
            }
            Some(_) => self.parse_unquoted_value(),
            None => bail!("expected a JSON value at byte {}", self.position),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Value> {
        self.expect('{')?;
        let mut object = Map::new();
        loop {
            self.skip_ignored();
            while self.consume(',') {
                self.skip_ignored();
            }
            if self.consume('}') || self.peek().is_none() {
                return Ok(Value::Object(object));
            }
            let start = self.position;
            let key = self.parse_key()?;
            let end = self.position;
            self.skip_ignored();
            if !self.consume(':')
                && let Some(quote) = self.swallowed(start, end)
            {
                // `"02 "text"`: the quote that closed the key opens the value.
                self.position = quote;
            }
            self.skip_ignored();
            if self.peek().is_none() {
                object.insert(key, Value::Null);
                return Ok(Value::Object(object));
            }
            object.insert(key, self.parse_value(depth)?);
            self.skip_ignored();
            self.consume(',');
        }
    }

    /// Where the closing quote of the quoted key between `start` and `end`
    /// is, when that key ends in whitespace and so swallowed its colon.
    fn swallowed(&self, start: usize, end: usize) -> Option<usize> {
        let key = &self.input[start..end];
        let opening = key.chars().next()?;
        let closing = quote_end(opening)?;
        let inner = key.strip_suffix(closing)?;
        inner
            .chars()
            .last()
            .filter(|last| last.is_whitespace())
            .map(|_| end - closing.len_utf8())
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value> {
        self.expect('[')?;
        let mut array = Vec::new();
        loop {
            self.skip_ignored();
            while self.consume(',') {
                self.skip_ignored();
            }
            if self.consume(']') || self.peek().is_none() {
                return Ok(Value::Array(array));
            }
            array.push(self.parse_value(depth)?);
            self.skip_ignored();
            self.consume(',');
        }
    }

    fn parse_key(&mut self) -> Result<String> {
        if self.peek().and_then(quote_end).is_some() {
            // Trimmed because a dropped colon leaves the space inside the key.
            return Ok(self.parse_string(false)?.trim().to_owned());
        }
        let start = self.position;
        while let Some(character) = self.peek() {
            if character.is_whitespace() || matches!(character, ':' | ',' | '}' | '{' | '[' | ']') {
                break;
            }
            self.advance();
        }
        let key = self.input[start..self.position].trim();
        if key.is_empty() {
            bail!("expected an object key at byte {start}");
        }
        Ok(key.to_owned())
    }

    /// A string from its opening quote. In a value (`heal`), a quote closes
    /// it only where the value could end; a key, short and quote-free, keeps
    /// the strict rule, since a key that swallowed its colon looks like a
    /// value holding a quote.
    fn parse_string(&mut self, heal: bool) -> Result<String> {
        let opening = self
            .advance()
            .ok_or_else(|| anyhow!("expected a string at byte {}", self.position))?;
        let closing = quote_end(opening)
            .ok_or_else(|| anyhow!("expected a quote at byte {}", self.position))?;
        let mut output = String::new();
        while let Some(character) = self.advance() {
            if character == closing {
                if closing == '\''
                    && output.chars().last().is_some_and(char::is_alphanumeric)
                    && self.peek().is_some_and(char::is_alphanumeric)
                {
                    output.push(character);
                    continue;
                }
                if closing != '\'' && heal && !self.closes_string() {
                    output.push(character);
                    continue;
                }
                return Ok(output);
            }
            if character == '\\' {
                self.parse_escape(&mut output);
            } else {
                output.push(character);
            }
        }
        Ok(output)
    }

    /// Whether a quote here ends the string. One glued to more text is
    /// content, as in `"that's "kindness.""`. A second quote is ambiguous,
    /// `"a""b"` being one value holding quotes or two that lost their comma:
    /// a value starting after the pair means this quote closes and only the
    /// comma was lost; anything else means the pair is inside the value.
    fn closes_string(&self) -> bool {
        match self.peek() {
            None => true,
            Some('"') => self
                .remaining()
                .chars()
                .nth(1)
                .is_some_and(|following| !ends_value(following)),
            Some(character) => ends_value(character),
        }
    }

    fn parse_escape(&mut self, output: &mut String) {
        let Some(escaped) = self.advance() else {
            return;
        };
        match escaped {
            'b' => output.push('\u{0008}'),
            'f' => output.push('\u{000c}'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'u' => output.push(self.parse_unicode_escape()),
            other => output.push(other),
        }
    }

    fn parse_unicode_escape(&mut self) -> char {
        let Some(high) = self.take_hex_quad() else {
            return char::REPLACEMENT_CHARACTER;
        };
        if !(0xd800..=0xdbff).contains(&high) {
            return char::from_u32(u32::from(high)).unwrap_or(char::REPLACEMENT_CHARACTER);
        }
        let saved = self.position;
        if self.consume('\\')
            && self.consume('u')
            && let Some(low @ 0xdc00..=0xdfff) = self.take_hex_quad()
        {
            let codepoint =
                0x1_0000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
            return char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER);
        }
        self.position = saved;
        char::REPLACEMENT_CHARACTER
    }

    fn take_hex_quad(&mut self) -> Option<u16> {
        let bytes = self.remaining().as_bytes().get(..4)?;
        if !bytes.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        self.position += 4;
        u16::from_str_radix(std::str::from_utf8(bytes).ok()?, 16).ok()
    }

    fn parse_number_or_string(&mut self) -> Value {
        let start = self.position;
        while self.peek().is_some_and(|character| {
            character.is_ascii_digit() || matches!(character, '+' | '-' | '.' | 'e' | 'E')
        }) {
            self.advance();
        }
        let token = &self.input[start..self.position];
        if let Ok(number) = token.parse::<Number>() {
            return Value::Number(number);
        }
        if let Ok(number) = token.parse::<f64>()
            && let Some(number) = Number::from_f64(number)
        {
            return Value::Number(number);
        }
        Value::String(token.to_owned())
    }

    fn parse_unquoted_value(&mut self) -> Result<Value> {
        let start = self.position;
        while let Some(character) = self.peek() {
            if matches!(character, ',' | ']' | '}') || self.starts_comment() {
                break;
            }
            self.advance();
        }
        let token = self.input[start..self.position].trim();
        if token.is_empty() {
            bail!("expected a JSON value at byte {start}");
        }
        Ok(match token.to_ascii_lowercase().as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "null" | "none" | "undefined" => Value::Null,
            _ => Value::String(token.to_owned()),
        })
    }

    fn skip_ignored(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.advance();
            }
            if self.remaining().starts_with("//") || self.remaining().starts_with('#') {
                while self.peek().is_some_and(|character| character != '\n') {
                    self.advance();
                }
                continue;
            }
            if self.remaining().starts_with("/*") {
                self.position += 2;
                match self.remaining().find("*/") {
                    Some(end) => self.position += end + 2,
                    None => self.position = self.input.len(),
                }
                continue;
            }
            break;
        }
    }

    fn starts_comment(&self) -> bool {
        self.remaining().starts_with("//")
            || self.remaining().starts_with("/*")
            || self.remaining().starts_with('#')
    }

    fn expect(&mut self, expected: char) -> Result<()> {
        if self.consume(expected) {
            Ok(())
        } else {
            bail!("expected `{expected}` at byte {}", self.position)
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.position += character.len_utf8();
        Some(character)
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.position..]
    }
}

/// Whether a value can't continue past `character`.
fn ends_value(character: char) -> bool {
    character.is_whitespace() || matches!(character, ',' | '}' | ']' | ':')
}

fn quote_end(character: char) -> Option<char> {
    match character {
        '"' | '\'' | '`' => Some(character),
        '“' => Some('”'),
        '‘' => Some('’'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(count: usize) -> Vec<String> {
        (1..=count).map(|number| format!("{number:02}")).collect()
    }

    fn all(answer: &str, count: usize) -> Vec<String> {
        answers(answer, &keys(count))
            .expect("an answer")
            .into_iter()
            .map(|text| text.expect("every text"))
            .collect()
    }

    #[test]
    fn reads_fenced_json_and_json_in_prose() {
        for answer in [
            "```json\n{\"01\": \"hello\", \"02\": \"world\"}\n```",
            "```\n{\"01\": \"hello\", \"02\": \"world\"}\n```",
            r#"Certainly. Here it is: {"01": "hello", "02": "world"} Enjoy!"#,
        ] {
            assert_eq!(all(answer, 2), ["hello", "world"], "{answer}");
        }
    }

    #[test]
    fn recovers_a_key_that_swallowed_its_colon() {
        let answer = "{\n  \"01\": \"first\",\n  \"02 \"second\"\n}";
        assert_eq!(all(answer, 2), ["first", "second"]);
    }

    #[test]
    fn keeps_unescaped_quotes_inside_a_value() {
        let answer = concat!(
            "{\n",
            "  \"01\": \"I don't think that's \"kindness.\"\",\n",
            "  \"02\": \"That thing you call \"kindness\"...\"\n",
            "}"
        );
        assert_eq!(
            all(answer, 2),
            [
                "I don't think that's \"kindness.\"",
                "That thing you call \"kindness\"..."
            ]
        );
    }

    #[test]
    fn keeps_adjacent_strings_apart_when_a_comma_is_missing() {
        assert_eq!(
            all(r#"{"01": "hello""02": "world"}"#, 2),
            ["hello", "world"]
        );
        assert_eq!(
            all(r#"{"01": "hello" "02": "world"}"#, 2),
            ["hello", "world"]
        );
        assert_eq!(
            all(r#"{"01": "that's "kindness."", "02": "ok"}"#, 2),
            ["that's \"kindness.\"", "ok"]
        );
    }

    #[test]
    fn reads_relaxed_syntax() {
        for answer in [
            "{01: 'hello', 02: 'world',}",
            "{“01”: “hello”, “02”: “world”}",
            "{\"01\": \"hello\", /* later */ \"02\": \"world\",}",
            "{\"1\": \"hello\", \"2\": \"world\"}",
        ] {
            assert_eq!(all(answer, 2), ["hello", "world"], "{answer}");
        }
    }

    #[test]
    fn closes_a_cut_off_answer() {
        assert_eq!(
            all("{\"01\": \"hello\", \"02\": \"world", 2),
            ["hello", "world"]
        );
    }

    #[test]
    fn keeps_escapes_and_unicode() {
        assert_eq!(
            all(r#"{01: "line\nfeed", 02: "日本 😀"}"#, 2),
            ["line\nfeed", "日本 😀"]
        );
    }

    #[test]
    fn keeps_the_texts_around_a_corrupt_one() {
        let answer = concat!(
            "{\"01\": \"first\", ",
            "\"02 \u{044d}\u{043a}\u{0440}\u{0430}\u{043d}\": \"!?\", ",
            "\"03\": \"  \", \"04\": \"fourth\"}"
        );
        assert_eq!(
            answers(answer, &keys(5)).expect("an answer"),
            [
                Some("first".to_owned()),
                None,
                None,
                Some("fourth".to_owned()),
                None
            ]
        );
    }

    #[test]
    fn an_answer_with_nothing_usable_fails() {
        assert!(answers(r#"{"result": ["hello"]}"#, &keys(1)).is_err());
        assert!(answers("hello, world", &keys(1)).is_err());
    }

    #[test]
    fn a_snippet_keeps_an_answer_on_one_line() {
        assert_eq!(
            snippet("{\n  \"01\":\t\"hello\"\n}"),
            r#"{\n  "01":\t"hello"\n}"#
        );
        assert_eq!(snippet("before\u{7}after"), r"before\u{0007}after");
        let long = snippet(&"x".repeat(2_500));
        assert!(long.starts_with(&"x".repeat(2_000)));
        assert!(long.ends_with("… (2500 bytes total)"), "{long}");
    }
}
