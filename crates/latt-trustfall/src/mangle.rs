//! Reversible encoding of lattice names into GraphQL identifiers.
//!
//! Kind and attr names may hold `-` (`spec-goal`), `:` (fused `reqs:requirement`)
//! or start with a digit, none of which a GraphQL name admits. The encoding is
//! lossless so a query result can be read back as the lattice name, and two
//! distinct lattice names never encode alike: `_` itself is escaped, so the
//! escape sequences cannot be forged by a name that already contains them.

/// Encode a lattice name as a GraphQL identifier.
///
/// `_` → `__`, `-` → `_h_`, `:` → `_c_`, `.` → `_p_`, any other non-identifier
/// character → `_uXXXX_` (its code point in hex), and a leading digit is
/// prefixed with `_d_`.
#[must_use]
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        out.push_str("_d_");
    }
    for c in name.chars() {
        match c {
            '_' => out.push_str("__"),
            '-' => out.push_str("_h_"),
            ':' => out.push_str("_c_"),
            '.' => out.push_str("_p_"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            c => out.push_str(&format!("_u{:x}_", c as u32)),
        }
    }
    out
}

/// Decode an identifier [`encode`] produced; `None` when the text is not one.
#[must_use]
pub fn decode(encoded: &str) -> Option<String> {
    let mut out = String::with_capacity(encoded.len());
    let mut rest = encoded.strip_prefix("_d_").unwrap_or(encoded);
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('_') {
            let (piece, tail) = if let Some(tail) = tail.strip_prefix('_') {
                ('_'.to_string(), tail)
            } else if let Some(tail) = tail.strip_prefix("h_") {
                ('-'.to_string(), tail)
            } else if let Some(tail) = tail.strip_prefix("c_") {
                (':'.to_string(), tail)
            } else if let Some(tail) = tail.strip_prefix("p_") {
                ('.'.to_string(), tail)
            } else {
                let tail = tail.strip_prefix('u')?;
                let (hex, tail) = tail.split_once('_')?;
                let code = u32::from_str_radix(hex, 16).ok()?;
                (char::from_u32(code)?.to_string(), tail)
            };
            out.push_str(&piece);
            rest = tail;
        } else {
            let mut chars = rest.chars();
            let c = chars.next()?;
            if !c.is_ascii_alphanumeric() {
                return None;
            }
            out.push(c);
            rest = chars.as_str();
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn round_trips_every_shape_a_profile_can_declare() {
        for name in [
            "req",
            "spec-goal",
            "reqs:requirement",
            "a_b",
            "a__b",
            "a_h_b",
            "3d-model",
            "x.y",
            "tést",
            "_",
            "",
        ] {
            let encoded = encode(name);
            assert!(
                encoded
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{name} → {encoded} is not an identifier"
            );
            assert!(
                !encoded.starts_with(|c: char| c.is_ascii_digit()),
                "{name} → {encoded} starts with a digit"
            );
            assert_eq!(decode(&encoded).as_deref(), Some(name), "{encoded}");
        }
    }

    #[test]
    fn distinct_names_never_collide() {
        let names = ["a-b", "a_h_b", "a__h__b", "a:b", "a_c_b", "a_b", "a__b"];
        let mut seen = std::collections::BTreeMap::new();
        for name in names {
            if let Some(other) = seen.insert(encode(name), name) {
                panic!("{name} and {other} both encode as {}", encode(name));
            }
        }
    }

    #[test]
    fn text_that_is_not_an_encoding_is_refused() {
        for text in ["a-b", "_x", "_q_", "_uzz_", "a b"] {
            assert_eq!(decode(text), None, "{text}");
        }
    }
}
