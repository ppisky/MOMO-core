//! TOML-based runtime configuration loading without freezing a product schema.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use thiserror::Error;
use toml::{Table, Value};

pub use momo_domain::SCHEMA_GENERATION;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration path must use the .toml extension: {0}")]
    InvalidExtension(PathBuf),
    #[error("configuration I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("invalid TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("configuration serialization failed: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("failed to persist configuration atomically: {0}")]
    Persist(#[from] tempfile::PersistError),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigDocument {
    values: Table,
}

impl ConfigDocument {
    #[must_use]
    pub const fn new(values: Table) -> Self {
        Self { values }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        ensure_toml_path(path)?;
        let text = fs::read_to_string(path)?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        Ok(Self::new(toml::from_str(text)?))
    }

    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        let mut text = toml::to_string_pretty(&self.values)?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        ensure_toml_path(path)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        let text = self.to_toml_string()?;
        temporary.write_all(text.as_bytes())?;
        temporary.as_file_mut().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        self.values.insert(key.into(), value)
    }

    #[must_use]
    pub const fn values(&self) -> &Table {
        &self.values
    }
}

fn ensure_toml_path(path: &Path) -> Result<(), ConfigError> {
    if path.extension().and_then(|value| value.to_str()) == Some("toml") {
        Ok(())
    } else {
        Err(ConfigError::InvalidExtension(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_unknown_configuration_fields() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("user.toml");
        let original: Table = toml::from_str(
            r#"
stream = true
[future_feature]
enabled = false
"#,
        )
        .expect("fixture TOML");
        ConfigDocument::new(original.clone())
            .save(&path)
            .expect("save configuration");
        let loaded = ConfigDocument::load(&path).expect("load configuration");
        assert_eq!(loaded.values(), &original);
    }

    #[test]
    fn rejects_non_toml_paths() {
        let result = ConfigDocument::load("config.json");
        assert!(matches!(result, Err(ConfigError::InvalidExtension(_))));
    }

    #[test]
    fn parses_and_formats_in_memory_configuration() {
        let document = ConfigDocument::parse("stream = true\n").expect("parse");
        assert_eq!(document.get("stream"), Some(&Value::Boolean(true)));
        assert_eq!(
            document.to_toml_string().expect("format"),
            "stream = true\n"
        );
    }
}
