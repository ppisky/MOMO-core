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
