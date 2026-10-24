// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of FileFormat.swift.
//!
//! The JSON files omaflake writes name their JSON Schema (plan B114): `"$schema":
//! "https://omaflake.dev/schema/<type>/v<n>.json"` – the type says what the file is, n its version, one count per type
//! (a change to instance.json needs no step for image.json). The schemas are in docs/schema/ and published on
//! omaflake.dev; a version, once published, stays there. An older file is brought up to date when it is read, step by
//! step (each step turns version n into n + 1), and the caller writes it back; a newer one (from a newer omaflake) is
//! refused instead of misread. A file without `$schema` is version 0 (written before B114). Each omaflake reads from a
//! `minimum` version on – a file older than that is named as too old (stores list them), not read.

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::{codes, Error, Result};
use crate::vm_err;

/// A migration step: turns the JSON object of version n into version n + 1 (Swift `FileFormat.Step`)
pub type Step = fn(&mut Map<String, Value>) -> Result<()>;

/// One kind of file omaflake writes, with its schema type and migration steps
#[derive(Debug, Clone, Copy)]
pub struct FileFormat {
    /// The schema's type: "image", "instance", "settings", …
    pub type_: &'static str,
    /// What the file is, for messages ("image.json")
    pub file: &'static str,
    /// The oldest version this omaflake reads (steps below it may be dropped in a later omaflake)
    pub minimum: usize,
    /// steps[n] turns version n into n + 1
    steps: &'static [Step],
}

impl FileFormat {
    /// Where the schemas live (docs/schema/ in this repository)
    pub const BASE: &'static str = "https://omaflake.dev/schema/";

    pub const fn new(type_: &'static str, file: &'static str, minimum: usize, steps: &'static [Step]) -> Self {
        FileFormat { type_, file, minimum, steps }
    }

    /// The version this omaflake writes
    pub fn current(&self) -> usize {
        self.steps.len()
    }

    /// The `$schema` this omaflake writes
    pub fn schema(&self) -> String {
        Self::schema_for(self.type_, self.current())
    }

    /// Swift `FileFormat.schema(type:version:)`
    pub fn schema_for(type_: &str, version: usize) -> String {
        format!("{}{}/v{}.json", Self::BASE, type_, version)
    }

    /// (type, version) of a schema URL of omaflake's, nil for anything else
    pub fn parse(schema: &str) -> Option<(String, usize)> {
        let rest = schema.strip_prefix(Self::BASE)?.strip_suffix(".json")?;
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() != 2 {
            return None;
        }
        let n: usize = parts[1].strip_prefix('v')?.parse().ok()?;
        if n < 1 {
            return None;
        }
        Some((parts[0].to_string(), n))
    }

    /// The version a file says it has: 0 without `$schema`; an error for another type, a foreign schema, or a version
    /// this omaflake does not read (too old or too new)
    pub fn version_of(&self, object: &Map<String, Value>) -> Result<usize> {
        let Some(value) = object.get("$schema") else {
            return Self::check(0, self.minimum, self.current(), self.file);
        };
        let parsed = value.as_str().and_then(Self::parse);
        let Some((type_, version)) = parsed else {
            let shown = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
            return Err(vm_err!("{}: unknown $schema {}", self.file, shown));
        };
        if type_ != self.type_ {
            return Err(vm_err!("{} is a {} file, not {}", self.file, type_, self.type_));
        }
        Self::check(version, self.minimum, self.current(), self.file)
    }

    /// `version` if this omaflake reads it, otherwise the error that says why not
    pub fn check(version: usize, minimum: usize, current: usize, file: &str) -> Result<usize> {
        if version > current {
            return Err(vm_err!(
                codes::NEWER_FILE,
                "{file} has version {version} – this omaflake reads up to {current}; update omaflake"
            ));
        }
        if version < minimum {
            return Err(vm_err!(
                codes::UNREADABLE_FILE,
                "{file} has version {version} – too old for this omaflake, which reads {minimum} to {current}"
            ));
        }
        Ok(version)
    }

    /// The file's JSON object at the current version; `migrated` (second value) when it was older.
    /// Swift returns the re-serialized `Data`; the Rust port hands back the object itself.
    pub fn upgrade(&self, data: &[u8]) -> Result<(Map<String, Value>, bool)> {
        let mut object = match serde_json::from_slice::<Value>(data) {
            Ok(Value::Object(o)) => o,
            _ => return Err(vm_err!("{} is not a JSON object", self.file)),
        };
        let version = self.version_of(&object)?;
        if version >= self.current() {
            return Ok((object, false));
        }
        for step in &self.steps[version..self.current()] {
            step(&mut object)?;
        }
        object.insert("$schema".into(), Value::String(self.schema()));
        Ok((object, true))
    }

    /// `data` as `T` at the current version; `migrated` tells the caller to write it back
    pub fn decode<T: DeserializeOwned>(&self, data: &[u8]) -> Result<(T, bool)> {
        let (object, migrated) = self.upgrade(data)?;
        match serde_json::from_value::<T>(Value::Object(object)) {
            Ok(v) => Ok((v, migrated)),
            Err(e) => Err(vm_err!("{} cannot be read: {}", self.file, e)),
        }
    }

    /// `value` as this file, with its `$schema` (first, as keys are sorted): pretty-printed, sorted keys, slashes not
    /// escaped (Swift `[.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]`)
    pub fn encode<T: Serialize>(&self, value: &T) -> Result<Vec<u8>> {
        let mut object = match serde_json::to_value(value)? {
            Value::Object(o) => o,
            _ => return Err(vm_err!("{}: not a JSON object", self.file)),
        };
        object.insert("$schema".into(), Value::String(self.schema()));
        Ok(crate::json::value_to_pretty(&Value::Object(object), false))
    }

    /// For steps: a key renamed (e.g. "flavor" → "recipe", plan B92); a file that has the new one already keeps it
    pub fn rename_key(old: &str, new: &str, object: &mut Map<String, Value>) {
        let Some(value) = object.remove(old) else { return };
        if object.contains_key(new) {
            return;
        }
        object.insert(new.to_string(), value);
    }

    /// Refuses to overwrite a file of a newer version (a newer omaflake wrote it; this one would lose what it added)
    pub fn check_writable(&self, path: &Path) -> Result<()> {
        let Ok(data) = std::fs::read(path) else { return Ok(()) };
        let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&data) else { return Ok(()) };
        self.version_of(&object).map(|_| ())
    }
}

/// A file of omaflake's that cannot be read (too old, too new, damaged) – listed with a warning instead of left out
/// silently (plan B114)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnreadableFile {
    /// The VM, image or recipe it belongs to
    pub name: String,
    pub reason: String,
}

impl UnreadableFile {
    pub fn new(name: impl Into<String>, reason: impl Into<String>) -> Self {
        UnreadableFile { name: name.into(), reason: reason.into() }
    }
}

/// Writes `data` atomically (temporary file next to it, then rename) – Swift `write(to:options: .atomic)`
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    use crate::error::IoContext;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, data).with_path(&tmp)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::from(e).context(format!("{}: ", path.display())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_or_foreign_schemas_are_refused() {
        let f = crate::models::InstanceConfig::FILE_FORMAT;
        assert!(f.upgrade(br#"{"$schema":"https://omaflake.dev/schema/instance/v99.json"}"#).is_err());
        assert_eq!(
            f.upgrade(br#"{"$schema":"https://omaflake.dev/schema/instance/v99.json"}"#).unwrap_err().code(),
            Some(codes::NEWER_FILE)
        );
        assert!(f.upgrade(br#"{"$schema":"https://omaflake.dev/schema/image/v1.json"}"#).is_err()); // another type
        assert!(f.upgrade(br#"{"$schema":"https://example.org/instance/v1.json"}"#).is_err());
        assert!(f.upgrade(b"[1]").is_err());
        assert_eq!(FileFormat::parse("https://omaflake.dev/schema/macos-vm/v12.json"), Some(("macos-vm".into(), 12)));
        assert_eq!(FileFormat::parse("https://omaflake.dev/schema/image/1.json"), None);
        assert_eq!(FileFormat::parse("https://omaflake.dev/schema/image/v0.json"), None);
    }

    fn noop(_: &mut Map<String, Value>) -> Result<()> {
        Ok(())
    }

    #[test]
    fn older_versions_are_read_within_the_minimum() {
        // A type that reads from version 1 on: version 0 is too old, 2 too new – both named, not read
        static STEPS: [Step; 1] = [noop];
        let f = FileFormat::new("image", "image.json", 1, &STEPS);
        let old = f.upgrade(br#"{"kind":"nixos"}"#).unwrap_err();
        assert_eq!(old.code(), Some(codes::UNREADABLE_FILE));
        assert!(f.upgrade(br#"{"$schema":"https://omaflake.dev/schema/image/v2.json"}"#).is_err());
        assert!(!f.upgrade(br#"{"$schema":"https://omaflake.dev/schema/image/v1.json"}"#).unwrap().1);
    }

    #[test]
    fn rename_keeps_a_new_key() {
        let mut o: Map<String, Value> = serde_json::from_str(r#"{"flavor":"a","recipe":"b"}"#).unwrap();
        FileFormat::rename_key("flavor", "recipe", &mut o);
        assert_eq!(o.get("recipe").unwrap(), "b");
        assert!(!o.contains_key("flavor"));
        let mut o: Map<String, Value> = serde_json::from_str(r#"{"flavor":"a"}"#).unwrap();
        FileFormat::rename_key("flavor", "recipe", &mut o);
        assert_eq!(o.get("recipe").unwrap(), "a");
    }
}
