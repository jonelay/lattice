//! Rust: inline `mod` chains of any depth, ending in an `fn` item at that
//! module's own level. Any other brace group — an `impl`, a `trait`, a fn
//! body, a macro, an `extern` block — is skipped whole, so nothing inside it
//! counts.

use super::{Cursor, ScanError, Scope, is_ident_continue, is_ident_start};

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    /// An opening bracket and the index of its matching close.
    Open(usize),
    Close,
    Punct(char),
}

pub(super) fn scan(text: &str) -> Result<Scope, ScanError> {
    let tokens = lex(text)?;
    let mut scope = Scope::default();
    read_items(&tokens, 0, tokens.len(), &mut scope);
    Ok(scope)
}

fn read_items(tokens: &[Token], mut index: usize, end: usize, scope: &mut Scope) {
    while index < end {
        match &tokens[index] {
            Token::Ident(word) if word == "mod" => {
                if let (Some(Token::Ident(name)), Some(Token::Open(close))) =
                    (tokens.get(index + 1), tokens.get(index + 2))
                {
                    let child = scope.children.entry(name.clone()).or_default();
                    read_items(tokens, index + 3, *close, child);
                    index = close + 1;
                    continue;
                }
                index += 1;
            }
            Token::Ident(word) if word == "fn" => {
                if let Some(Token::Ident(name)) = tokens.get(index + 1) {
                    scope.names.insert(name.clone());
                }
                index += 1;
            }
            Token::Open(close) => index = close + 1,
            _ => index += 1,
        }
    }
}

fn lex(text: &str) -> Result<Vec<Token>, ScanError> {
    let mut cursor = Cursor::new(text);
    let mut tokens = Vec::new();
    let mut open: Vec<(usize, char, u32)> = Vec::new();
    while let Some(c) = cursor.peek() {
        let line = cursor.line;
        match c {
            '/' if cursor.peek_at(1) == Some('/') => cursor.skip_line(),
            '/' if cursor.peek_at(1) == Some('*') => block_comment(&mut cursor)?,
            '"' => {
                cursor.bump();
                string(&mut cursor, line)?;
            }
            '\'' => char_or_lifetime(&mut cursor, line)?,
            '(' | '[' | '{' => {
                cursor.bump();
                open.push((tokens.len(), c, line));
                tokens.push(Token::Open(0));
            }
            ')' | ']' | '}' => {
                cursor.bump();
                let expected = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match open.pop() {
                    Some((at, opener, _)) if opener == expected => {
                        tokens[at] = Token::Open(tokens.len());
                        tokens.push(Token::Close);
                    }
                    _ => return Err(ScanError::new(line, format!("unbalanced bracket '{c}'"))),
                }
            }
            c if is_ident_start(c) => {
                let word = cursor.ident();
                if literal_after_prefix(&mut cursor, &word, line)? {
                    continue;
                }
                if word == "r" && cursor.peek() == Some('#') {
                    // A raw identifier: `r#match` reads as `match`.
                    cursor.bump();
                    tokens.push(Token::Ident(cursor.ident()));
                    continue;
                }
                tokens.push(Token::Ident(word));
            }
            c if c.is_whitespace() => {
                cursor.bump();
            }
            c => {
                cursor.bump();
                tokens.push(Token::Punct(c));
            }
        }
    }
    if let Some((_, opener, line)) = open.pop() {
        return Err(ScanError::new(
            line,
            format!("unbalanced bracket '{opener}'"),
        ));
    }
    Ok(tokens)
}

/// Consume a literal that `word` prefixes (`b"…"`, `r#"…"#`, `b'x'`, `c"…"`).
/// True when one was consumed; false when `word` is an identifier.
fn literal_after_prefix(cursor: &mut Cursor, word: &str, line: u32) -> Result<bool, ScanError> {
    match (word, cursor.peek()) {
        ("b" | "c", Some('"')) => {
            cursor.bump();
            string(cursor, line)?;
            Ok(true)
        }
        ("b", Some('\'')) => {
            char_or_lifetime(cursor, line)?;
            Ok(true)
        }
        ("r" | "br" | "cr", Some('"' | '#')) => {
            let mut hashes = 0;
            while cursor.peek_at(hashes) == Some('#') {
                hashes += 1;
            }
            if cursor.peek_at(hashes) != Some('"') {
                // `r#ident`: a raw identifier, not a raw string.
                return Ok(false);
            }
            for _ in 0..=hashes {
                cursor.bump();
            }
            raw_string(cursor, hashes, line)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn string(cursor: &mut Cursor, line: u32) -> Result<(), ScanError> {
    while let Some(c) = cursor.bump() {
        match c {
            '\\' => {
                cursor.bump();
            }
            '"' => return Ok(()),
            _ => {}
        }
    }
    Err(ScanError::new(line, "unterminated string"))
}

fn raw_string(cursor: &mut Cursor, hashes: usize, line: u32) -> Result<(), ScanError> {
    while let Some(c) = cursor.bump() {
        if c == '"' && (0..hashes).all(|offset| cursor.peek_at(offset) == Some('#')) {
            for _ in 0..hashes {
                cursor.bump();
            }
            return Ok(());
        }
    }
    Err(ScanError::new(line, "unterminated raw string"))
}

fn block_comment(cursor: &mut Cursor) -> Result<(), ScanError> {
    let line = cursor.line;
    let mut depth = 0usize;
    while cursor.peek().is_some() {
        if cursor.starts_with("/*") {
            cursor.bump();
            cursor.bump();
            depth += 1;
        } else if cursor.starts_with("*/") {
            cursor.bump();
            cursor.bump();
            depth -= 1;
            if depth == 0 {
                return Ok(());
            }
        } else {
            cursor.bump();
        }
    }
    Err(ScanError::new(line, "unterminated block comment"))
}

/// A char literal (`'x'`, `'\n'`, `'\u{1F600}'`) or a lifetime or label
/// (`'a`, `'static`): a lifetime is a quote whose next-but-one char is not a
/// closing quote.
fn char_or_lifetime(cursor: &mut Cursor, line: u32) -> Result<(), ScanError> {
    cursor.bump();
    match cursor.peek() {
        Some('\\') => {
            while let Some(c) = cursor.bump() {
                match c {
                    '\\' => {
                        cursor.bump();
                    }
                    '\'' => return Ok(()),
                    '\n' => break,
                    _ => {}
                }
            }
            Err(ScanError::new(line, "unterminated char literal"))
        }
        Some(_) if cursor.peek_at(1) == Some('\'') => {
            cursor.bump();
            cursor.bump();
            Ok(())
        }
        Some(c) if is_ident_continue(c) => {
            cursor.ident();
            Ok(())
        }
        _ => Ok(()),
    }
}
