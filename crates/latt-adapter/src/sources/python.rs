//! Python: a definition counts at module top level, or directly in the body of
//! a top-level class — the rule `adapters/_python_tests.py` applies with the
//! AST. The scanner reads logical lines: strings, comments, backslash
//! continuation and bracket continuation never start one, so text that only
//! looks like a definition is not one.

use super::{Cursor, ScanError, Scope, is_ident_start};

/// How much of a logical line the index needs: where it starts, how far it is
/// indented, its leading words, and whether its last token is a `:`.
#[derive(Debug)]
struct Logical {
    indent: usize,
    words: Vec<String>,
    words_open: bool,
    ends_with_colon: bool,
}

enum Token {
    Word(String),
    Colon,
    Other,
}

impl Logical {
    fn mark(&mut self, token: Token) {
        self.ends_with_colon = matches!(token, Token::Colon);
        match token {
            Token::Word(word) if self.words_open && self.words.len() < 3 => self.words.push(word),
            _ => self.words_open = false,
        }
    }

    /// The name a `def`, `async def` or (when `classes`) `class` line defines.
    fn defines(&self, classes: bool) -> Option<&str> {
        match self.words.as_slice() {
            [def, name, ..] if def == "def" => Some(name),
            [r#async, def, name] if r#async == "async" && def == "def" => Some(name),
            [class, name, ..] if classes && class == "class" => Some(name),
            _ => None,
        }
    }
}

pub(super) fn scan(text: &str) -> Result<Scope, ScanError> {
    let lines = logical_lines(text)?;
    let mut scope = Scope::default();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        index += 1;
        if line.indent != 0 {
            continue;
        }
        let Some(name) = line.defines(true) else {
            continue;
        };
        scope.names.insert(name.to_owned());
        if line.words[0] != "class" {
            continue;
        }
        // A class whose body shares its header line (`class A: pass`) still
        // exists; it defines no methods.
        let mut class = Scope::default();
        if !line.ends_with_colon {
            scope.children.insert(name.to_owned(), class);
            continue;
        }
        if let Some(body) = lines
            .get(index)
            .map(|line| line.indent)
            .filter(|indent| *indent > 0)
        {
            while let Some(member) = lines.get(index).filter(|line| line.indent > 0) {
                if member.indent == body
                    && let Some(method) = member.defines(false)
                {
                    class.names.insert(method.to_owned());
                }
                index += 1;
            }
        }
        scope.children.insert(name.to_owned(), class);
    }
    Ok(scope)
}

fn logical_lines(text: &str) -> Result<Vec<Logical>, ScanError> {
    let mut cursor = Cursor::new(text);
    let mut lines = Vec::new();
    let mut current: Option<Logical> = None;
    let mut open: Vec<(char, u32)> = Vec::new();
    loop {
        if current.is_none() {
            let mut indent = 0;
            loop {
                match cursor.peek() {
                    Some(' ') => indent += 1,
                    Some('\t') => indent = (indent / 8 + 1) * 8,
                    Some('\x0c') => indent = 0,
                    _ => break,
                }
                cursor.bump();
            }
            match cursor.peek() {
                None => break,
                Some('\n' | '\r') => {
                    cursor.bump();
                    continue;
                }
                Some('#') => {
                    cursor.skip_line();
                    continue;
                }
                Some(_) => {
                    current = Some(Logical {
                        indent,
                        words: Vec::new(),
                        words_open: true,
                        ends_with_colon: false,
                    });
                }
            }
        }
        let Some(logical) = current.as_mut() else {
            continue;
        };
        let Some(c) = cursor.peek() else {
            break;
        };
        let line = cursor.line;
        match c {
            '#' => cursor.skip_line(),
            '\\' if matches!(
                (cursor.peek_at(1), cursor.peek_at(2)),
                (Some('\n'), _) | (Some('\r'), Some('\n'))
            ) =>
            {
                while cursor.bump() != Some('\n') {}
            }
            '\n' => {
                cursor.bump();
                if open.is_empty() {
                    lines.extend(current.take());
                }
            }
            '\'' | '"' => {
                string(&mut cursor, c, false, line)?;
                logical.mark(Token::Other);
            }
            c if is_ident_start(c) => {
                let word = cursor.ident();
                match cursor.peek() {
                    Some(quote @ ('\'' | '"')) if is_prefix(&word) => {
                        string(&mut cursor, quote, is_formatted(&word), line)?;
                        logical.mark(Token::Other);
                    }
                    _ => logical.mark(Token::Word(word)),
                }
            }
            '(' | '[' | '{' => {
                cursor.bump();
                open.push((c, line));
                logical.mark(Token::Other);
            }
            ')' | ']' | '}' => {
                cursor.bump();
                let expected = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match open.pop() {
                    Some((opener, _)) if opener == expected => {}
                    _ => return Err(ScanError::new(line, format!("unbalanced bracket '{c}'"))),
                }
                logical.mark(Token::Other);
            }
            ':' => {
                cursor.bump();
                logical.mark(Token::Colon);
            }
            c if c.is_whitespace() => {
                cursor.bump();
            }
            _ => {
                cursor.bump();
                logical.mark(Token::Other);
            }
        }
    }
    if let Some((opener, line)) = open.pop() {
        return Err(ScanError::new(
            line,
            format!("unbalanced bracket '{opener}'"),
        ));
    }
    lines.extend(current);
    Ok(lines)
}

/// True when `word` is a string prefix: `r`, `u`, `f`, `b`, `t` and their
/// two-letter combinations, in either case.
fn is_prefix(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "r" | "u" | "f" | "b" | "t" | "br" | "rb" | "fr" | "rf" | "tr" | "rt"
    )
}

fn is_formatted(prefix: &str) -> bool {
    prefix.chars().any(|c| matches!(c, 'f' | 'F' | 't' | 'T'))
}

/// Consume a string from its opening quote. `formatted` strings have their
/// replacement fields scanned as code, so a field may reuse the enclosing
/// quote (PEP 701).
fn string(cursor: &mut Cursor, quote: char, formatted: bool, line: u32) -> Result<(), ScanError> {
    let triple = cursor.peek_at(1) == Some(quote) && cursor.peek_at(2) == Some(quote);
    let closer: String = std::iter::repeat_n(quote, if triple { 3 } else { 1 }).collect();
    for _ in 0..closer.len() {
        cursor.bump();
    }
    loop {
        let Some(c) = cursor.peek() else {
            return Err(ScanError::new(line, "unterminated string"));
        };
        if c == '\\' {
            cursor.bump();
            cursor.bump();
            continue;
        }
        if cursor.starts_with(&closer) {
            for _ in 0..closer.len() {
                cursor.bump();
            }
            return Ok(());
        }
        if c == '\n' && !triple {
            return Err(ScanError::new(line, "unterminated string"));
        }
        cursor.bump();
        if formatted && c == '{' {
            if cursor.peek() == Some('{') {
                cursor.bump();
            } else {
                replacement_field(cursor, line)?;
            }
        }
    }
}

/// Consume an f-string replacement field after its `{`, through its `}`.
fn replacement_field(cursor: &mut Cursor, line: u32) -> Result<(), ScanError> {
    let mut depth = 0usize;
    loop {
        let Some(c) = cursor.peek() else {
            return Err(ScanError::new(line, "unterminated string"));
        };
        match c {
            '\'' | '"' => string(cursor, c, false, cursor.line)?,
            // A comment in a multi-line field ends at the newline, which the
            // enclosing string's own scan then sees.
            '#' => {
                while cursor.peek().is_some_and(|c| c != '\n') {
                    cursor.bump();
                }
            }
            c if is_ident_start(c) => {
                let word = cursor.ident();
                if let Some(quote @ ('\'' | '"')) = cursor.peek()
                    && is_prefix(&word)
                {
                    string(cursor, quote, is_formatted(&word), cursor.line)?;
                }
            }
            '(' | '[' | '{' => {
                cursor.bump();
                depth += 1;
            }
            ')' | ']' => {
                cursor.bump();
                depth = depth.saturating_sub(1);
            }
            '}' => {
                cursor.bump();
                if depth == 0 {
                    return Ok(());
                }
                depth -= 1;
            }
            ':' if depth == 0 => {
                cursor.bump();
                return format_spec(cursor, line);
            }
            _ => {
                cursor.bump();
            }
        }
    }
}

/// Consume a format spec through the `}` that closes its field; a nested
/// `{…}` is a replacement field of its own.
fn format_spec(cursor: &mut Cursor, line: u32) -> Result<(), ScanError> {
    loop {
        match cursor.bump() {
            None => return Err(ScanError::new(line, "unterminated string")),
            Some('{') => replacement_field(cursor, line)?,
            Some('}') => return Ok(()),
            Some(_) => {}
        }
    }
}
