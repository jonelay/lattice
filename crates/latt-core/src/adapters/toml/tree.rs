//! A parsed TOML file as values that keep where they were written.
//!
//! `toml::Value` drops source positions, which would leave every node and finding
//! at line 0 and no finding able to show the entry it is about.
//! Each [`Val`] carries its byte span, from which [`Text`] gives the line and
//! the entry exactly as the register wrote it.

use std::ops::Range;

/// A TOML value and the bytes it was parsed from, when the parser kept them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Val {
    pub(crate) kind: Kind,
    pub(crate) span: Option<Range<usize>>,
}

/// A table's entries, sorted by key as `toml::Value`'s are: keys are visited in
/// key order, not in the order the file wrote them.
///
/// A sorted `Vec`, not a `BTreeMap`: a register row holds a handful of keys,
/// and a `BTreeMap` allocates a whole eleven-slot node for them, which over a
/// large register set the adapter's peak memory. Keys are unique, as the parser
/// refuses a repeated one.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Map(Vec<(String, Val)>);

impl Map {
    pub(crate) fn get(&self, key: &str) -> Option<&Val> {
        self.0
            .binary_search_by(|(at, _)| at.as_str().cmp(key))
            .ok()
            .map(|index| &self.0[index].1)
    }
}

impl Map {
    /// Sort by key and drop spare capacity, which would otherwise hold much of
    /// what the sorted `Vec` saves; a caller that sized `entries` exactly
    /// spares the reallocation.
    fn from_entries(mut entries: Vec<(String, Val)>) -> Self {
        entries.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
        entries.shrink_to_fit();
        Self(entries)
    }
}

impl FromIterator<(String, Val)> for Map {
    fn from_iter<I: IntoIterator<Item = (String, Val)>>(entries: I) -> Self {
        Self::from_entries(entries.into_iter().collect())
    }
}

impl<'m> IntoIterator for &'m Map {
    type Item = &'m (String, Val);
    type IntoIter = std::slice::Iter<'m, (String, Val)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl std::ops::Index<&str> for Map {
    type Output = Val;

    fn index(&self, key: &str) -> &Val {
        self.get(key).expect("no such key in table")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(String),
    Array(Vec<Val>),
    Table(Map),
}

impl Val {
    pub(crate) fn as_table(&self) -> Option<&Map> {
        match &self.kind {
            Kind::Table(table) => Some(table),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match &self.kind {
            Kind::String(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn type_name(&self) -> &'static str {
        match self.kind {
            Kind::String(_) => "string",
            Kind::Integer(_) => "integer",
            Kind::Float(_) => "float",
            Kind::Boolean(_) => "boolean",
            Kind::Datetime(_) => "datetime",
            Kind::Array(_) => "array",
            Kind::Table(_) => "table",
        }
    }
}

/// A file's text, for turning a span into a line and back into source.
pub(crate) struct Text<'t> {
    text: &'t str,
    newlines: Vec<usize>,
}

impl<'t> Text<'t> {
    pub(crate) fn new(text: &'t str) -> Self {
        let newlines = text
            .bytes()
            .enumerate()
            .filter_map(|(at, byte)| (byte == b'\n').then_some(at))
            .collect();
        Self { text, newlines }
    }

    /// Return the one-based line a span starts on, or 0 without a span: the file
    /// as a whole, as the interface document spells it.
    pub(crate) fn line(&self, span: Option<&Range<usize>>) -> u32 {
        span.map_or(0, |span| {
            let line = self.newlines.partition_point(|&at| at < span.start) + 1;
            u32::try_from(line).unwrap_or(u32::MAX)
        })
    }

    /// Return the value as written, on one line, or `None` without a span.
    ///
    /// Runs of whitespace collapse to one space, so a multi-line entry cannot
    /// break a plain finding; a longer one keeps its first `limit` characters
    /// and gains an ellipsis.
    pub(crate) fn source(&self, span: Option<&Range<usize>>, limit: usize) -> Option<String> {
        let written = self.text.get(span?.clone())?;
        let collapsed = written.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.chars().count() <= limit {
            return Some(collapsed);
        }
        let mut cut: String = collapsed.chars().take(limit).collect();
        cut.push('…');
        Some(cut)
    }
}

/// Parse a file, keeping spans; the error is the parser's own, with its span.
///
/// The parser's tree is consumed as it is converted: each part's span is read
/// before the part is moved, and string values move rather than copy, so the
/// two trees are never both whole in memory.
pub(crate) fn parse(text: &str) -> Result<Val, toml_edit::TomlError> {
    let document = toml_edit::ImDocument::parse(text)?;
    Ok(table(document.into_table()))
}

fn table(table: toml_edit::Table) -> Val {
    let span = table.span();
    let mut entries = Vec::with_capacity(table.len());
    entries.extend(
        table
            .into_iter()
            .filter_map(|(key, item)| item_val(item).map(|val| (key.as_str().to_owned(), val))),
    );
    Val {
        kind: Kind::Table(Map::from_entries(entries)),
        span,
    }
}

fn item_val(item: toml_edit::Item) -> Option<Val> {
    match item {
        toml_edit::Item::None => None,
        toml_edit::Item::Value(value) => Some(value_val(value)),
        toml_edit::Item::Table(inner) => Some(table(inner)),
        toml_edit::Item::ArrayOfTables(rows) => {
            let span = rows.span();
            Some(Val {
                kind: Kind::Array(rows.into_iter().map(table).collect()),
                span,
            })
        }
    }
}

fn value_val(value: toml_edit::Value) -> Val {
    use toml_edit::Value as V;
    let span = value.span();
    let kind = match value {
        V::String(v) => Kind::String(v.into_value()),
        V::Integer(v) => Kind::Integer(*v.value()),
        V::Float(v) => Kind::Float(*v.value()),
        V::Boolean(v) => Kind::Boolean(*v.value()),
        V::Datetime(v) => Kind::Datetime(v.value().to_string()),
        V::Array(entries) => Kind::Array(entries.into_iter().map(value_val).collect()),
        V::InlineTable(inline) => Kind::Table(
            inline
                .into_iter()
                .map(|(key, value)| (key.as_str().to_owned(), value_val(value)))
                .collect(),
        ),
    };
    Val { kind, span }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Text, Val, parse};

    #[test]
    fn rows_keys_and_entries_keep_their_lines() {
        let text = "title = \"t\"\n\n[[item]]\nid = \"A\"\nrefs = [\n  \"B\",\n  { glob = \"c\" },\n]\n\n[[item]]\nid = \"D\"\n";
        let lines = Text::new(text);
        let root = parse(text).unwrap();
        let root = root.as_table().unwrap();
        assert_eq!(lines.line(root["title"].span.as_ref()), 1);
        let Kind::Array(rows) = &root["item"].kind else {
            panic!("rows")
        };
        assert_eq!(
            lines.line(rows[0].span.as_ref()),
            3,
            "a row is at its header"
        );
        assert_eq!(lines.line(rows[1].span.as_ref()), 10);
        let row = rows[0].as_table().unwrap();
        let Kind::Array(refs) = &row["refs"].kind else {
            panic!("refs")
        };
        assert_eq!(lines.line(row["refs"].span.as_ref()), 5);
        assert_eq!(lines.line(refs[0].span.as_ref()), 6);
        assert_eq!(lines.line(refs[1].span.as_ref()), 7);
        assert_eq!(
            lines.source(refs[1].span.as_ref(), 80).as_deref(),
            Some("{ glob = \"c\" }"),
            "an entry reads as the register wrote it"
        );
    }

    /// The conversion as it read the parser's tree by reference, before it
    /// consumed it: the oracle the consuming conversion must agree with.
    fn by_reference(table: &toml_edit::Table) -> Val {
        fn scalar(value: &toml_edit::Value) -> Val {
            use toml_edit::Value as V;
            let kind = match value {
                V::String(v) => Kind::String(v.value().clone()),
                V::Integer(v) => Kind::Integer(*v.value()),
                V::Float(v) => Kind::Float(*v.value()),
                V::Boolean(v) => Kind::Boolean(*v.value()),
                V::Datetime(v) => Kind::Datetime(v.value().to_string()),
                V::Array(entries) => Kind::Array(entries.iter().map(scalar).collect()),
                V::InlineTable(inline) => Kind::Table(
                    inline
                        .iter()
                        .map(|(key, v)| (key.to_owned(), scalar(v)))
                        .collect(),
                ),
            };
            Val {
                kind,
                span: value.span(),
            }
        }
        let map = table
            .iter()
            .filter_map(|(key, item)| {
                let val = match item {
                    toml_edit::Item::None => return None,
                    toml_edit::Item::Value(v) => scalar(v),
                    toml_edit::Item::Table(inner) => by_reference(inner),
                    toml_edit::Item::ArrayOfTables(rows) => Val {
                        kind: Kind::Array(rows.iter().map(by_reference).collect()),
                        span: rows.span(),
                    },
                };
                Some((key.to_owned(), val))
            })
            .collect();
        Val {
            kind: Kind::Table(map),
            span: table.span(),
        }
    }

    #[test]
    fn consuming_the_parser_tree_keeps_every_value_and_span() {
        let text = "title = \"t\"\nwhen = 1979-05-27T07:32:00Z\nn = 3\nf = 1.5\nb = true\n\
                    a.b.c = \"dotted\"\nempty = []\n\n[meta]\nnested = { x = [1, { y = \"z\" }] }\n\n\
                    [meta.deeper]\nk = 'literal'\n\n[[item]]\nid = \"A\"\nrefs = [\n  \"B\",\n  \
                    { glob = \"c\" },\n]\n\n[[item]]\nid = \"D\"\n[item.sub]\nq = -1\n";
        let oracle = by_reference(toml_edit::ImDocument::parse(text).unwrap().as_table());
        assert_eq!(parse(text).unwrap(), oracle);
        let Kind::Table(root) = &oracle.kind else {
            panic!("root")
        };
        // Every value written out has a span, so equality covers spans too; an
        // implicit table (`a` of `a.b.c`) has none of its own.
        assert!(root["a"].span.is_none());
        assert!(
            root.into_iter()
                .filter(|(key, _)| key.as_str() != "a")
                .all(|(_, val)| val.span.is_some()),
            "the oracle saw spans"
        );
    }

    #[test]
    fn source_is_one_line_and_bounded() {
        let text = "a = [\n  1,\n    2,\n]\n";
        let lines = Text::new(text);
        let root = parse(text).unwrap();
        let span = root.as_table().unwrap()["a"].span.clone();
        assert_eq!(
            lines.source(span.as_ref(), 80).as_deref(),
            Some("[ 1, 2, ]")
        );
        assert_eq!(lines.source(span.as_ref(), 4).as_deref(), Some("[ 1,…"));
        assert_eq!(lines.line(None), 0);
        assert_eq!(lines.source(None, 80), None);
    }

    /// A row is at its `[[table]]` header even when comments and blank lines
    /// precede it, a nested table and an inline table are at their own lines,
    /// and a key in a `[table]` section is at its own line.
    #[test]
    fn tables_and_sections_are_at_their_headers() {
        let text = "# c\n\n[[item]]\nid = \"A\"\n[item.meta]\nk = 1\n\n# gap\n[[item]]\nid = \"B\"\nt = { x = 1 }\n\n[settings]\nname = \"s\"\n";
        let lines = Text::new(text);
        let root = parse(text).unwrap();
        let root = root.as_table().unwrap();
        let Kind::Array(rows) = &root["item"].kind else {
            panic!("rows")
        };
        assert_eq!(lines.line(rows[0].span.as_ref()), 3);
        assert_eq!(lines.line(rows[1].span.as_ref()), 9);
        let first = rows[0].as_table().unwrap();
        assert_eq!(lines.line(first["meta"].span.as_ref()), 5);
        let meta = first["meta"].as_table().unwrap();
        assert_eq!(lines.line(meta["k"].span.as_ref()), 6);
        let second = rows[1].as_table().unwrap();
        assert_eq!(lines.line(second["t"].span.as_ref()), 11);
        let settings = root["settings"].as_table().unwrap();
        assert_eq!(lines.line(root["settings"].span.as_ref()), 13);
        assert_eq!(lines.line(settings["name"].span.as_ref()), 14);
    }

    mod properties {
        use proptest::prelude::*;

        use super::super::Text;

        /// A word: no whitespace, may be multi-byte.
        fn word() -> impl Strategy<Value = String> {
            "[a-z0-9é→=\\[\\]{}\",]{1,6}"
        }

        /// A run of whitespace, possibly with newlines.
        fn gap() -> impl Strategy<Value = String> {
            "[ \t\n\r]{1,4}"
        }

        proptest! {
            /// A span starting anywhere on line `n` of a text built from known
            /// lines is on line `n`, one-based.
            #[test]
            fn a_span_is_on_the_line_its_start_is_on(
                written in proptest::collection::vec("[^\n]{0,8}", 1..8),
                pick in any::<prop::sample::Index>(),
                column in any::<prop::sample::Index>(),
            ) {
                let text = written.join("\n");
                let at = pick.index(written.len());
                let before: usize = written[..at].iter().map(|line| line.len() + 1).sum();
                let offsets: Vec<usize> = written[at]
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain([written[at].len()])
                    .collect();
                let start = before + offsets[column.index(offsets.len())];
                let lines = Text::new(&text);
                prop_assert_eq!(lines.line(Some(&(start..text.len()))) as usize, at + 1);
            }

            /// The source of a span is its words joined by one space, cut at
            /// `limit` characters with an ellipsis when longer.
            #[test]
            fn source_is_the_words_on_one_line_within_the_limit(
                lead in "[ \t\n]{0,3}",
                words in proptest::collection::vec((word(), gap()), 1..8),
                limit in 0usize..40,
            ) {
                let mut text = lead.clone();
                for (word, gap) in &words {
                    text.push_str(word);
                    text.push_str(gap);
                }
                let joined = words.iter().map(|(w, _)| w.as_str()).collect::<Vec<_>>().join(" ");
                let expected = if joined.chars().count() <= limit {
                    joined
                } else {
                    joined.chars().take(limit).chain(['…']).collect()
                };
                let lines = Text::new(&text);
                prop_assert_eq!(lines.source(Some(&(0..text.len())), limit), Some(expected));
            }
        }
    }
}
