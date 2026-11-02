//! Go: a top-level `func` names a definition. A `func` followed by `(` is a
//! method receiver or a function type or literal and names nothing; anything
//! inside a bracket group is below the top level.

use super::{Cursor, ScanError, Scope, is_ident_start};

pub(super) fn scan(text: &str) -> Result<Scope, ScanError> {
    let mut cursor = Cursor::new(text);
    let mut scope = Scope::default();
    let mut open: Vec<(char, u32)> = Vec::new();
    let mut after_func = false;
    while let Some(c) = cursor.peek() {
        let line = cursor.line;
        if c.is_whitespace() {
            cursor.bump();
            continue;
        }
        let was_after_func = std::mem::take(&mut after_func);
        match c {
            '/' if cursor.peek_at(1) == Some('/') => {
                cursor.skip_line();
                after_func = was_after_func;
            }
            '/' if cursor.peek_at(1) == Some('*') => {
                cursor.bump();
                cursor.bump();
                loop {
                    if cursor.starts_with("*/") {
                        cursor.bump();
                        cursor.bump();
                        break;
                    }
                    if cursor.bump().is_none() {
                        return Err(ScanError::new(line, "unterminated block comment"));
                    }
                }
                after_func = was_after_func;
            }
            '"' | '\'' => {
                cursor.bump();
                quoted(&mut cursor, c, line)?;
            }
            '`' => {
                cursor.bump();
                loop {
                    match cursor.bump() {
                        Some('`') => break,
                        Some(_) => {}
                        None => return Err(ScanError::new(line, "unterminated raw string")),
                    }
                }
            }
            '(' | '[' | '{' => {
                cursor.bump();
                open.push((c, line));
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
            }
            c if is_ident_start(c) => {
                let word = cursor.ident();
                if open.is_empty() {
                    if was_after_func {
                        scope.names.insert(word);
                    } else {
                        after_func = word == "func";
                    }
                }
            }
            _ => {
                cursor.bump();
            }
        }
    }
    if let Some((opener, line)) = open.pop() {
        return Err(ScanError::new(
            line,
            format!("unbalanced bracket '{opener}'"),
        ));
    }
    Ok(scope)
}

fn quoted(cursor: &mut Cursor, quote: char, line: u32) -> Result<(), ScanError> {
    let what = if quote == '"' {
        "unterminated string"
    } else {
        "unterminated rune literal"
    };
    while let Some(c) = cursor.bump() {
        match c {
            '\\' => {
                cursor.bump();
            }
            '\n' => break,
            c if c == quote => return Ok(()),
            _ => {}
        }
    }
    Err(ScanError::new(line, what))
}
