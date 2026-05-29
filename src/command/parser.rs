//! Tokeniser for the `:`-command line.
//!
//! Splits a typed line (sans leading `:`) into a command name and zero or
//! more whitespace-separated arguments. Double-quoted spans allow embedded
//! whitespace; backslash escapes a quote or backslash inside them.
//! No glob expansion, no variable substitution — this is a deliberately
//! minimal lexer.

/// Parses a typed command line (without the leading `:`) into a
/// command name and a list of positional arguments.
///
/// # Errors
///
/// - [`ParseError::Empty`] — empty or whitespace-only input.
/// - [`ParseError::UnterminatedQuote`] — opening `"` with no matching close.
pub fn parse(line: &str) -> Result<(String, Vec<String>), ParseError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }

    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escape = false;
    // Tracks whether the current token has started — distinct from
    // `current.is_empty()` so an empty quoted span `""` still pushes one
    // (deliberately empty) arg.
    let mut token_started = false;

    for c in trimmed.chars() {
        if escape {
            current.push(c);
            escape = false;
            token_started = true;
            continue;
        }
        if c == '\\' && in_quotes {
            escape = true;
            continue;
        }
        if c == '"' {
            in_quotes = !in_quotes;
            token_started = true;
            continue;
        }
        if c.is_whitespace() && !in_quotes {
            if token_started {
                tokens.push(std::mem::take(&mut current));
                token_started = false;
            }
            continue;
        }
        current.push(c);
        token_started = true;
    }

    if in_quotes {
        return Err(ParseError::UnterminatedQuote);
    }
    if token_started {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return Err(ParseError::Empty);
    }
    let name = tokens.remove(0);
    Ok((name, tokens))
}

/// Failure modes for [`parse`].
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Line was empty or only whitespace.
    Empty,
    /// Line had an opening `"` with no matching close.
    UnterminatedQuote,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("empty command"),
            Self::UnterminatedQuote => f.write_str("unterminated quote"),
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(line: &str) -> (String, Vec<String>) {
        parse(line).expect("expected parse to succeed")
    }

    #[test]
    fn parses_command_with_no_args() {
        assert_eq!(parse_ok("quit"), ("quit".to_string(), vec![]));
    }

    #[test]
    fn parses_command_with_one_arg() {
        let (cmd, args) = parse_ok("feed top");
        assert_eq!(cmd, "feed");
        assert_eq!(args, vec!["top".to_string()]);
    }

    #[test]
    fn parses_multiple_args() {
        let (cmd, args) = parse_ok("filter author dang");
        assert_eq!(cmd, "filter");
        assert_eq!(args, vec!["author".to_string(), "dang".to_string()]);
    }

    #[test]
    fn parses_quoted_arg_with_spaces() {
        let (cmd, args) = parse_ok(r#"search "hello world""#);
        assert_eq!(cmd, "search");
        assert_eq!(args, vec!["hello world".to_string()]);
    }

    #[test]
    fn parses_escaped_quote_inside_quoted_span() {
        let (cmd, args) = parse_ok(r#"x "a\"b""#);
        assert_eq!(cmd, "x");
        assert_eq!(args, vec![r#"a"b"#.to_string()]);
    }

    #[test]
    fn parses_escaped_backslash_inside_quoted_span() {
        let (cmd, args) = parse_ok(r#"x "a\\b""#);
        assert_eq!(cmd, "x");
        assert_eq!(args, vec![r"a\b".to_string()]);
    }

    #[test]
    fn trims_leading_and_trailing_whitespace() {
        assert_eq!(
            parse_ok("  feed   top  "),
            ("feed".into(), vec!["top".into()])
        );
    }

    #[test]
    fn collapses_internal_whitespace_between_tokens() {
        assert_eq!(
            parse_ok("a\t\tb   c"),
            ("a".into(), vec!["b".into(), "c".into()])
        );
    }

    #[test]
    fn rejects_empty_input() {
        assert_eq!(parse(""), Err(ParseError::Empty));
        assert_eq!(parse("   "), Err(ParseError::Empty));
        assert_eq!(parse("\t\n  "), Err(ParseError::Empty));
    }

    #[test]
    fn rejects_unterminated_quote() {
        assert_eq!(parse(r#"search "foo"#), Err(ParseError::UnterminatedQuote));
    }

    #[test]
    fn empty_quoted_span_yields_empty_arg() {
        let (cmd, args) = parse_ok(r#"x """#);
        assert_eq!(cmd, "x");
        assert_eq!(args, vec!["".to_string()]);
    }

    #[test]
    fn display_messages_are_meaningful() {
        assert_eq!(ParseError::Empty.to_string(), "empty command");
        assert_eq!(
            ParseError::UnterminatedQuote.to_string(),
            "unterminated quote"
        );
    }
}
