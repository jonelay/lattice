//! Reading the resolved profile document the core hands every adapter.

use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;

/// Bound on one subprocess an adapter runs when the profile sets no
/// `adapter.command_timeout_seconds`.
#[doc(hidden)]
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// Read the resolved profile document at `path` into the adapter's typed view.
///
/// See [`resolved_profile_from_slice`] for the checks and the error text.
///
/// # Errors
///
/// The file cannot be read, or [`resolved_profile_from_slice`] refuses its bytes.
///
/// ```
/// let path = std::env::temp_dir().join(format!("latt-adapter-profile-{}.json", std::process::id()));
/// std::fs::write(&path, r#"{"resolved_schema": "1", "name": "demo"}"#)?;
/// let profile: serde_json::Value = latt_adapter::load_resolved_profile(&path)?;
/// std::fs::remove_file(&path)?;
/// assert_eq!(profile["name"], "demo");
///
/// let error = latt_adapter::load_resolved_profile::<serde_json::Value>(&path).unwrap_err();
/// assert!(error.starts_with(&format!("could not load profile {}: ", path.display())), "{error}");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn load_resolved_profile<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|error| failure(path, error))?;
    resolved_profile_from_slice(&bytes, path)
}

/// Check that `bytes` are a resolved profile document and deserialize them as `T`.
///
/// The marker is checked first, on an untyped pass, so a JSON document without
/// `"resolved_schema": "1"`, or naming a future schema, is named as not a
/// resolved profile document whatever else is wrong with it. Bytes that are not
/// JSON at all, such as a profile's YAML source, fail that pass with serde's
/// message.
/// The typed pass then reads the same bytes again rather than converting the
/// untyped value: only a pass over the text keeps serde's ` at line N column M`.
/// `T` should not declare `resolved_schema`; it is checked here.
///
/// # Errors
///
/// Every refusal is one line, `could not load profile {path}: ` followed by
/// serde's message, or by `not a resolved profile document` when the marker
/// is missing or names another schema.
///
/// ```
/// use std::path::Path;
///
/// #[derive(serde::Deserialize)]
/// struct View { name: String }
///
/// let path = Path::new("profile.json");
/// let view: View = latt_adapter::resolved_profile_from_slice(
///     br#"{"resolved_schema": "1", "name": "demo"}"#, path)?;
/// assert_eq!(view.name, "demo");
///
/// let unmarked = latt_adapter::resolved_profile_from_slice::<View>(br#"{"name": "demo"}"#, path);
/// assert_eq!(
///     unmarked.err().as_deref(),
///     Some("could not load profile profile.json: not a resolved profile document"),
/// );
///
/// let mistyped = latt_adapter::resolved_profile_from_slice::<View>(
///     br#"{"resolved_schema": "1", "name": 7}"#, path);
/// assert!(mistyped.err().is_some_and(|e| e.ends_with("at line 1 column 34")));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn resolved_profile_from_slice<T: DeserializeOwned>(
    bytes: &[u8],
    path: &Path,
) -> Result<T, String> {
    let untyped: Value = serde_json::from_slice(bytes).map_err(|error| failure(path, error))?;
    if untyped.get("resolved_schema").and_then(Value::as_str) != Some("1") {
        return Err(failure(path, "not a resolved profile document"));
    }
    serde_json::from_slice(bytes).map_err(|error| failure(path, error))
}

fn failure(path: &Path, error: impl std::fmt::Display) -> String {
    format!("could not load profile {}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde::Deserialize;

    use super::resolved_profile_from_slice;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Adapter {
        #[serde(default)]
        command_timeout_seconds: Option<u64>,
    }

    #[derive(Debug, Deserialize)]
    struct View {
        adapter: Adapter,
    }

    fn read(text: &str) -> Result<View, String> {
        resolved_profile_from_slice(text.as_bytes(), Path::new("p.json"))
    }

    #[test]
    fn load_resolved_profile_refuses_a_missing_or_wrong_marker() {
        let adapter = r#""adapter":{"command_timeout_seconds":5}"#;
        let view = read(&format!(r#"{{"resolved_schema":"1",{adapter}}}"#)).unwrap();
        assert_eq!(view.adapter.command_timeout_seconds, Some(5));
        for marker in ["", r#""resolved_schema":"2","#, r#""resolved_schema":1,"#] {
            assert_eq!(
                read(&format!("{{{marker}{adapter}}}")).unwrap_err(),
                "could not load profile p.json: not a resolved profile document",
                "{marker}"
            );
        }
    }

    #[test]
    fn load_resolved_profile_names_the_marker_before_a_typed_fault() {
        let error = read(r#"{"adapter":{"command_timeout_seconds":"5"}}"#).unwrap_err();
        assert!(
            error.ends_with("not a resolved profile document"),
            "{error}"
        );
    }

    #[test]
    fn load_resolved_profile_keeps_serde_field_and_position() {
        let error = read("{\"resolved_schema\":\"1\",\n\"adapter\":{\"comand\":1}}").unwrap_err();
        assert!(
            error.starts_with("could not load profile p.json: unknown field `comand`"),
            "{error}"
        );
        assert!(error.contains(" at line 2 column "), "{error}");
        let error = read("resolved_schema: \"1\"").unwrap_err();
        assert!(
            error.starts_with("could not load profile p.json: expected value at line 1"),
            "{error}"
        );
    }
}
