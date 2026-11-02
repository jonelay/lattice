//! A map held as one vector of entries sorted by key.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};

/// A map held as one vector of entries sorted by key, each key once.
///
/// It answers as a [`BTreeMap`] does: lookup by key, iteration in key order, and a
/// repeated key replacing the earlier value. A node's attrs and attr locations are
/// mostly one or two entries, and a `BTreeMap` allocates a 632-byte leaf for each of
/// them; at 16,384 nodes those leaves set the peak memory of a validation on both
/// sides of the adapter boundary. This map allocates one vector sized to its entries.
/// Insertion is linear in the entries after the new key, so it suits small maps.
///
/// It serializes as the JSON object a `BTreeMap` writes, byte for byte, and
/// deserializes from a JSON object with a `BTreeMap`'s rule for a repeated key: the
/// last value wins.
///
/// ```
/// use latt_adapter::SortedMap;
///
/// let mut map = SortedMap::new();
/// map.insert("b".to_owned(), 2);
/// map.insert("a".to_owned(), 1);
/// assert_eq!(map.insert("b".to_owned(), 3), Some(2));
/// assert_eq!(map.get("b"), Some(&3));
/// assert_eq!(serde_json::to_string(&map)?, r#"{"a":1,"b":3}"#);
///
/// let read: SortedMap<String, u8> = serde_json::from_str(r#"{"b":1,"a":2,"b":3}"#)?;
/// assert_eq!(read.iter().collect::<Vec<_>>(), [(&"a".to_owned(), &2), (&"b".to_owned(), &3)]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct SortedMap<K, V> {
    entries: Vec<(K, V)>,
}

impl<K, V> SortedMap<K, V> {
    /// Construct an empty map; it allocates nothing until the first insert.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Return the number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the map holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate the entries in ascending key order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&K, &V)> + ExactSizeIterator {
        self.entries.iter().map(|(key, value)| (key, value))
    }

    /// Return the entries as a slice, in ascending key order.
    #[must_use]
    pub fn as_slice(&self) -> &[(K, V)] {
        &self.entries
    }
}

impl<K: Ord, V> SortedMap<K, V> {
    /// Find `key`'s index, or the index it would be inserted at.
    fn search<Q>(&self, key: &Q) -> Result<usize, usize>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.entries
            .binary_search_by(|(probe, _)| probe.borrow().cmp(key))
    }

    /// Return the value stored under `key`.
    #[must_use]
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.search(key).ok().map(|at| &self.entries[at].1)
    }

    /// True when the map holds `key`.
    #[must_use]
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.search(key).is_ok()
    }

    /// Store `value` under `key`, and return the value it replaces.
    ///
    /// The vector grows to exactly one entry on the first insert and doubles after
    /// that, so a one-entry map holds one entry's worth of memory where a plain
    /// `push` would reserve four.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.search(&key) {
            Ok(at) => Some(std::mem::replace(&mut self.entries[at].1, value)),
            Err(at) => {
                if self.entries.len() == self.entries.capacity() {
                    self.entries.reserve_exact(self.entries.len().max(1));
                }
                self.entries.insert(at, (key, value));
                None
            }
        }
    }
}

impl<K, V> Default for SortedMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for SortedMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for SortedMap<K, V> {
    /// Collect entries into a map; of several entries with one key, the last wins.
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut entries: Vec<(K, V)> = iter.into_iter().collect();
        // Stable, so the entries of one key stay in arrival order and the last of
        // each run is the one to keep.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                std::mem::swap(later, kept);
            }
            same
        });
        entries.shrink_to_fit();
        Self { entries }
    }
}

impl<K: Ord, V> From<BTreeMap<K, V>> for SortedMap<K, V> {
    /// Take a `BTreeMap`'s entries, which are already sorted and unique.
    fn from(map: BTreeMap<K, V>) -> Self {
        let mut entries = Vec::with_capacity(map.len());
        entries.extend(map);
        Self { entries }
    }
}

impl<K, V> IntoIterator for SortedMap<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;

    /// Move the entries out in ascending key order.
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<K: Serialize, V: Serialize> Serialize for SortedMap<K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (key, value) in &self.entries {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de, K, V> Deserialize<'de> for SortedMap<K, V>
where
    K: Ord + Deserialize<'de>,
    V: Deserialize<'de>,
{
    /// Read a map, and nothing else: a sequence of pairs is refused, as a
    /// `BTreeMap` refuses it. A repeated key keeps its last value.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<K, V>(PhantomData<(K, V)>);

        impl<'de, K, V> Visitor<'de> for Entries<K, V>
        where
            K: Ord + Deserialize<'de>,
            V: Deserialize<'de>,
        {
            type Value = SortedMap<K, V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut map = SortedMap::new();
                while let Some((key, value)) = access.next_entry()? {
                    map.insert(key, value);
                }
                map.entries.shrink_to_fit();
                Ok(map)
            }
        }

        deserializer.deserialize_map(Entries(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;
    use serde_json::Value;

    use super::SortedMap;

    /// Keys from a small alphabet, so lists repeat keys often.
    fn entries() -> impl Strategy<Value = Vec<(String, u8)>> {
        proptest::collection::vec(("[a-d]{0,2}", any::<u8>()), 0..12)
    }

    /// Write `entries` as one JSON object, in list order and with every repeat.
    fn object_text(entries: &[(String, u8)]) -> String {
        let fields: Vec<String> = entries
            .iter()
            .map(|(key, value)| format!("{}:{value}", Value::from(key.as_str())))
            .collect();
        format!("{{{}}}", fields.join(","))
    }

    proptest! {
        /// The law: built from the same entries, by insertion or by collecting, the
        /// map holds what a `BTreeMap` holds, finds what it finds, writes the JSON it
        /// writes, and reads a JSON object with repeated keys as it reads it.
        #[test]
        fn answers_as_a_btreemap(list in entries(), absent in "[a-e]{0,3}") {
            let model: BTreeMap<String, u8> = list.iter().cloned().collect();

            let mut inserted = SortedMap::new();
            let mut replaced = BTreeMap::new();
            for (key, value) in list.iter().cloned() {
                prop_assert_eq!(inserted.insert(key.clone(), value), replaced.insert(key, value));
            }
            let collected: SortedMap<String, u8> = list.iter().cloned().collect();
            let converted = SortedMap::from(model.clone());
            prop_assert_eq!(&inserted, &collected);
            prop_assert_eq!(&inserted, &converted);

            let pairs: Vec<(&String, &u8)> = inserted.iter().collect();
            prop_assert_eq!(pairs, model.iter().collect::<Vec<_>>());
            prop_assert_eq!(inserted.len(), model.len());
            prop_assert_eq!(inserted.is_empty(), model.is_empty());
            for key in list.iter().map(|(key, _)| key).chain([&absent]) {
                prop_assert_eq!(inserted.get(key.as_str()), model.get(key.as_str()));
                prop_assert_eq!(inserted.contains_key(key.as_str()), model.contains_key(key.as_str()));
            }
            prop_assert_eq!(format!("{inserted:?}"), format!("{model:?}"));
            prop_assert_eq!(
                serde_json::to_string(&inserted).unwrap(),
                serde_json::to_string(&model).unwrap()
            );

            let text = object_text(&list);
            let read: SortedMap<String, u8> = serde_json::from_str(&text).unwrap();
            let expected: BTreeMap<String, u8> = serde_json::from_str(&text).unwrap();
            prop_assert_eq!(read.iter().collect::<Vec<_>>(), expected.iter().collect::<Vec<_>>());
            prop_assert_eq!(read.into_iter().collect::<Vec<_>>(), expected.into_iter().collect::<Vec<_>>());
        }
    }

    #[test]
    fn a_one_entry_map_holds_one_entry() {
        let mut map = SortedMap::new();
        map.insert("t".to_owned(), 1_u8);
        assert_eq!(map.entries.capacity(), 1);
        let read: SortedMap<String, u8> = serde_json::from_str(r#"{"a":1,"b":2,"c":3}"#).unwrap();
        assert_eq!(read.entries.capacity(), 3);
    }

    #[test]
    fn only_an_object_is_read() {
        for text in [r#"[["a",1]]"#, "null", "1", r#"{"a":"x"}"#] {
            let typed = serde_json::from_str::<SortedMap<String, u8>>(text);
            let model = serde_json::from_str::<BTreeMap<String, u8>>(text);
            assert!(typed.is_err() && model.is_err(), "{text}");
        }
    }
}
