//! Safe creation and extraction of MOMO portable containers.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;

pub const CONTAINER_ENCODING: &str = "tar.zstd";
pub const FORMAT_NAME: &str = "momo-container";
pub const FORMAT_VERSION: u32 = 2;
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;
pub const DEFAULT_MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub package_type: PackageType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sequence: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_sequence: Option<i64>,
    #[serde(default)]
    pub module_definitions: Vec<ModuleDefinition>,
    pub modules: Vec<ModuleEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deletions: Vec<DeletionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionMetadata>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageType {
    #[default]
    Snapshot,
    Incremental,
    Deletion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleDefinition {
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    pub import_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeletionRecord {
    pub module: String,
    pub object_id: String,
    pub revision: i64,
    pub change_sequence: i64,
    pub deleted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptionMetadata {
    pub profile: String,
    pub payload_path: String,
    pub associated_data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleEntry {
    pub module: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtractionLimits {
    pub max_entries: usize,
    pub max_unpacked_bytes: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_unpacked_bytes: DEFAULT_MAX_UNPACKED_BYTES,
        }
    }
}

#[derive(Debug, Error)]
pub enum MocError {
    #[error("MOC I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("MOC manifest serialization failed: {0}")]
    ManifestEncode(#[from] toml::ser::Error),
    #[error("MOC manifest parsing failed: {0}")]
    ManifestDecode(#[from] toml::de::Error),
    #[error("invalid or unsafe archive path: {0}")]
    UnsafePath(PathBuf),
    #[error("source is outside the declared root: {0}")]
    SourceOutsideRoot(PathBuf),
    #[error("duplicate archive path: {0}")]
    DuplicatePath(String),
    #[error("unsupported MOC format")]
    UnsupportedFormat,
    #[error("unsupported MOC format version {found}; this build supports version {supported}")]
    UnsupportedFormatVersion { found: u32, supported: u32 },
    #[error("MOC v1 must be migrated explicitly before import")]
    LegacyMigrationRequired,
    #[error("invalid MOC manifest: {0}")]
    InvalidManifest(String),
    #[error("MOC extraction limit exceeded")]
    LimitExceeded,
    #[error("manifest entry does not match payload: {0}")]
    Integrity(String),
    #[error("directory traversal failed: {0}")]
    Walk(#[from] walkdir::Error),
}

pub fn create(
    output: impl AsRef<Path>,
    source_root: impl AsRef<Path>,
    modules: &[(String, PathBuf)],
) -> Result<Manifest, MocError> {
    create_with_encryption(output, source_root, modules, None)
}

pub fn create_with_encryption(
    output: impl AsRef<Path>,
    source_root: impl AsRef<Path>,
    modules: &[(String, PathBuf)],
    encryption: Option<EncryptionMetadata>,
) -> Result<Manifest, MocError> {
    let output = output.as_ref();
    let source_root = source_root.as_ref().canonicalize()?;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut module_definitions = Vec::new();
    let mut seen_modules = HashSet::new();

    for (module, relative_source) in modules {
        validate_relative(relative_source)?;
        validate_native_v2_module_id(module)?;
        if !seen_modules.insert(module.clone()) {
            return Err(MocError::InvalidManifest(format!(
                "duplicate module definition: {module}"
            )));
        }
        module_definitions.push(module_definition(module, relative_source)?);
        let source = source_root.join(relative_source).canonicalize()?;
        if !source.starts_with(&source_root) {
            return Err(MocError::SourceOutsideRoot(source));
        }
        if source.is_dir() {
            for item in WalkDir::new(&source).follow_links(false) {
                let item = item?;
                if item.file_type().is_symlink() || !item.file_type().is_file() {
                    continue;
                }
                let relative = item
                    .path()
                    .strip_prefix(&source_root)
                    .map_err(|_| MocError::SourceOutsideRoot(item.path().to_path_buf()))?;
                push_entry(module, relative, item.path(), &mut entries, &mut seen)?;
            }
        } else {
            push_entry(module, relative_source, &source, &mut entries, &mut seen)?;
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    module_definitions.sort_by_key(|module| (module.import_order, module.id.clone()));

    let manifest = Manifest {
        format: FORMAT_NAME.to_owned(),
        format_version: FORMAT_VERSION,
        created_at: Utc::now(),
        package_type: PackageType::Snapshot,
        base_sequence: None,
        through_sequence: None,
        module_definitions,
        modules: entries,
        deletions: Vec::new(),
        encryption,
    };
    validate_v2_manifest(&manifest)?;
    write_container(output, &source_root, &manifest)?;
    Ok(manifest)
}

pub fn inspect(input: impl AsRef<Path>) -> Result<Manifest, MocError> {
    let decoder = zstd::Decoder::new(File::open(input)?)?;
    let mut archive = tar::Archive::new(decoder);
    for item in archive.entries()? {
        let mut item = item?;
        let path = item.path()?.into_owned();
        validate_relative(&path)?;
        if normalized_path(&path)? != "manifest.toml" {
            continue;
        }
        if item.size() > 1024 * 1024 {
            return Err(MocError::LimitExceeded);
        }
        let mut text = String::new();
        item.read_to_string(&mut text)?;
        let manifest: Manifest = toml::from_str(&text)?;
        if manifest.format != FORMAT_NAME {
            return Err(MocError::UnsupportedFormat);
        }
        if manifest.format_version != FORMAT_VERSION {
            return Err(MocError::UnsupportedFormatVersion {
                found: manifest.format_version,
                supported: FORMAT_VERSION,
            });
        }
        validate_v2_manifest(&manifest)?;
        return Ok(manifest);
    }
    Err(MocError::UnsupportedFormat)
}

pub fn extract(
    input: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    limits: ExtractionLimits,
) -> Result<Manifest, MocError> {
    extract_version(input, destination, limits, FORMAT_VERSION)
}

fn extract_version(
    input: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    limits: ExtractionLimits,
    expected_version: u32,
) -> Result<Manifest, MocError> {
    let destination = destination.as_ref();
    fs::create_dir_all(destination)?;
    let decoder = zstd::Decoder::new(File::open(input)?)?;
    let mut archive = tar::Archive::new(decoder);
    let mut seen = HashSet::new();
    let mut manifest = None;
    let mut extracted = Vec::new();
    let mut total_size = 0_u64;

    for (index, item) in archive.entries()?.enumerate() {
        if index >= limits.max_entries {
            return Err(MocError::LimitExceeded);
        }
        let mut item = item?;
        let path = item.path()?.into_owned();
        validate_relative(&path)?;
        let normalized = normalized_path(&path)?;
        if !seen.insert(normalized.clone()) {
            return Err(MocError::DuplicatePath(normalized));
        }
        if !item.header().entry_type().is_file() {
            return Err(MocError::UnsafePath(path));
        }
        let size = item.size();
        total_size = total_size
            .checked_add(size)
            .ok_or(MocError::LimitExceeded)?;
        if total_size > limits.max_unpacked_bytes {
            return Err(MocError::LimitExceeded);
        }
        if normalized == "manifest.toml" {
            let mut text = String::new();
            item.read_to_string(&mut text)?;
            manifest = Some(toml::from_str::<Manifest>(&text)?);
            continue;
        }
        let target = destination.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        item.unpack(&target)?;
        extracted.push((normalized, target, size));
    }

    let manifest = manifest.ok_or(MocError::UnsupportedFormat)?;
    if manifest.format != FORMAT_NAME {
        return Err(MocError::UnsupportedFormat);
    }
    if manifest.format_version != expected_version {
        return Err(MocError::UnsupportedFormatVersion {
            found: manifest.format_version,
            supported: expected_version,
        });
    }
    validate_manifest(&manifest)?;
    verify_entries(&manifest, &extracted)?;
    Ok(manifest)
}

fn write_container(output: &Path, source_root: &Path, manifest: &Manifest) -> Result<(), MocError> {
    let output_parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent)?;
    let temporary = tempfile::NamedTempFile::new_in(output_parent)?;
    {
        let encoder = zstd::Encoder::new(temporary.as_file(), 9)?;
        let mut archive = tar::Builder::new(encoder.auto_finish());
        let manifest_text = toml::to_string_pretty(manifest)?;
        append_bytes(&mut archive, "manifest.toml", manifest_text.as_bytes())?;
        for entry in &manifest.modules {
            archive.append_path_with_name(source_root.join(&entry.path), &entry.path)?;
        }
        archive.finish()?;
    }
    temporary.persist(output).map_err(|error| error.error)?;
    Ok(())
}

fn push_entry(
    module: &str,
    relative: &Path,
    source: &Path,
    entries: &mut Vec<ModuleEntry>,
    seen: &mut HashSet<String>,
) -> Result<(), MocError> {
    validate_relative(relative)?;
    let path = normalized_path(relative)?;
    if path == "manifest.toml" || !seen.insert(path.clone()) {
        return Err(MocError::DuplicatePath(path));
    }
    let bytes = fs::read(source)?;
    entries.push(ModuleEntry {
        module: module.to_owned(),
        path,
        size: u64::try_from(bytes.len()).map_err(|_| MocError::LimitExceeded)?,
        sha256: hex::encode(Sha256::digest(&bytes)),
    });
    Ok(())
}

fn verify_entries(
    manifest: &Manifest,
    extracted: &[(String, PathBuf, u64)],
) -> Result<(), MocError> {
    let expected_paths = validate_manifest_paths(manifest)?;
    let actual_paths = extracted
        .iter()
        .map(|(path, _, _)| path.clone())
        .collect::<HashSet<_>>();
    if expected_paths != actual_paths {
        return Err(MocError::Integrity("payload path set".to_owned()));
    }
    for expected in &manifest.modules {
        let (_, path, size) = extracted
            .iter()
            .find(|(name, _, _)| name == &expected.path)
            .ok_or_else(|| MocError::Integrity(expected.path.clone()))?;
        let bytes = fs::read(path)?;
        let actual_hash = hex::encode(Sha256::digest(&bytes));
        if *size != expected.size || actual_hash != expected.sha256 {
            return Err(MocError::Integrity(expected.path.clone()));
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<(), MocError> {
    if manifest.format_version == FORMAT_VERSION {
        validate_v2_manifest(manifest)
    } else {
        Err(MocError::UnsupportedFormatVersion {
            found: manifest.format_version,
            supported: FORMAT_VERSION,
        })
    }
}

fn validate_v2_manifest(manifest: &Manifest) -> Result<(), MocError> {
    validate_manifest_paths(manifest)?;
    let mut definitions = HashMap::new();
    for definition in &manifest.module_definitions {
        validate_native_v2_module_id(&definition.id)?;
        validate_manifest_path(&definition.path)?;
        if let Some((path, dependencies, import_order)) = known_module_layout(&definition.id)
            && (definition.path != path
                || definition.dependencies != dependencies
                || definition.import_order != import_order)
        {
            return Err(MocError::InvalidManifest(format!(
                "module {} does not use its canonical v2 layout",
                definition.id
            )));
        }
        let mut dependencies = HashSet::new();
        for dependency in &definition.dependencies {
            validate_module_id(dependency)?;
            if dependency == &definition.id || !dependencies.insert(dependency) {
                return Err(MocError::InvalidManifest(format!(
                    "invalid dependency for module {}",
                    definition.id
                )));
            }
        }
        if definitions.insert(&definition.id, definition).is_some() {
            return Err(MocError::InvalidManifest(format!(
                "duplicate module definition: {}",
                definition.id
            )));
        }
    }
    for entry in &manifest.modules {
        let definition = definitions.get(&entry.module).ok_or_else(|| {
            MocError::InvalidManifest(format!(
                "payload {} references undeclared module {}",
                entry.path, entry.module
            ))
        })?;
        if !entry_belongs_to_module(entry, definition) {
            return Err(MocError::InvalidManifest(format!(
                "payload {} is outside module {}",
                entry.path, entry.module
            )));
        }
    }
    for deletion in &manifest.deletions {
        if !definitions.contains_key(&deletion.module)
            || deletion.object_id.trim().is_empty()
            || deletion.revision < 1
            || deletion.change_sequence < 1
        {
            return Err(MocError::InvalidManifest(
                "invalid deletion record".to_owned(),
            ));
        }
    }
    match manifest.package_type {
        PackageType::Snapshot => {
            if manifest.base_sequence.is_some()
                || manifest.through_sequence.is_some()
                || !manifest.deletions.is_empty()
            {
                return Err(MocError::InvalidManifest(
                    "snapshot packages cannot carry sequence bounds or deletions".to_owned(),
                ));
            }
        }
        PackageType::Incremental => {
            validate_sequence_range(manifest)?;
            if manifest.modules.is_empty() && manifest.deletions.is_empty() {
                return Err(MocError::InvalidManifest(
                    "incremental packages cannot be empty".to_owned(),
                ));
            }
        }
        PackageType::Deletion => {
            validate_sequence_range(manifest)?;
            if !manifest.modules.is_empty() || manifest.deletions.is_empty() {
                return Err(MocError::InvalidManifest(
                    "deletion packages contain deletion records and no payload files".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_native_v2_module_id(module: &str) -> Result<(), MocError> {
    validate_module_id(module)?;
    if matches!(module, "character" | "conversation") {
        return Err(MocError::InvalidManifest(format!(
            "legacy module id {module} is not valid in native v2"
        )));
    }
    Ok(())
}

fn validate_sequence_range(manifest: &Manifest) -> Result<(), MocError> {
    match (manifest.base_sequence, manifest.through_sequence) {
        (Some(base), Some(through)) if base >= 0 && through > base => Ok(()),
        _ => Err(MocError::InvalidManifest(
            "incremental and deletion packages require an increasing sequence range".to_owned(),
        )),
    }
}

fn validate_module_id(module: &str) -> Result<(), MocError> {
    if module.is_empty()
        || !module
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(MocError::InvalidManifest(format!(
            "invalid module id: {module}"
        )));
    }
    Ok(())
}

fn validate_manifest_path(value: &str) -> Result<(), MocError> {
    if value.contains('\\') {
        return Err(MocError::UnsafePath(PathBuf::from(value)));
    }
    let path = Path::new(value);
    validate_relative(path)?;
    if normalized_path(path)? != value || value == "manifest.toml" {
        return Err(MocError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

fn entry_belongs_to_module(entry: &ModuleEntry, definition: &ModuleDefinition) -> bool {
    entry.path == definition.path
        || entry
            .path
            .strip_prefix(&definition.path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn known_module_layout(module: &str) -> Option<(&'static str, Vec<String>, u32)> {
    let (path, dependencies, import_order): (&str, &[&str], u32) = match module {
        "encrypted-container" => ("private", &[], 0),
        "config" => ("config", &[], 10),
        "characters" => ("characters", &[], 20),
        "conversations" => ("conversations", &["characters"], 30),
        "memory" => ("memory", &[], 40),
        "semantic_graph" => ("semantic_graph", &[], 50),
        _ => return None,
    };
    Some((
        path,
        dependencies
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        import_order,
    ))
}

fn module_definition(module: &str, source: &Path) -> Result<ModuleDefinition, MocError> {
    validate_module_id(module)?;
    let source = normalized_path(source)?;
    if let Some((path, dependencies, import_order)) = known_module_layout(module) {
        if source != path {
            return Err(MocError::InvalidManifest(format!(
                "module {module} must use {path}, not {source}"
            )));
        }
        Ok(ModuleDefinition {
            id: module.to_owned(),
            path: path.to_owned(),
            dependencies,
            import_order,
        })
    } else {
        Ok(ModuleDefinition {
            id: module.to_owned(),
            path: source,
            dependencies: Vec::new(),
            import_order: 1_000,
        })
    }
}

fn validate_manifest_paths(manifest: &Manifest) -> Result<HashSet<String>, MocError> {
    let mut paths = HashSet::with_capacity(manifest.modules.len());
    for entry in &manifest.modules {
        validate_manifest_path(&entry.path)?;
        let normalized = entry.path.clone();
        if !paths.insert(normalized.clone()) {
            return Err(MocError::DuplicatePath(normalized));
        }
    }
    Ok(paths)
}

fn append_bytes<W: io::Write>(
    archive: &mut tar::Builder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), io::Error> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    archive.append_data(&mut header, path, bytes)
}

fn validate_relative(path: &Path) -> Result<(), MocError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(MocError::UnsafePath(path.to_path_buf()));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(MocError::UnsafePath(path.to_path_buf()));
        }
    }
    Ok(())
}

fn normalized_path(path: &Path) -> Result<String, MocError> {
    validate_relative(path)?;
    Ok(path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_raw_container(output: &Path, manifest: &Manifest, payloads: &[(&str, &[u8])]) {
        let encoder =
            zstd::Encoder::new(File::create(output).expect("create archive"), 1).expect("encoder");
        let mut archive = tar::Builder::new(encoder.auto_finish());
        let manifest_text = toml::to_string_pretty(manifest).expect("manifest");
        append_bytes(&mut archive, "manifest.toml", manifest_text.as_bytes())
            .expect("append manifest");
        for (path, bytes) in payloads {
            append_bytes(&mut archive, path, bytes).expect("append payload");
        }
        archive.finish().expect("finish archive");
    }

    #[test]
    fn creates_and_extracts_verified_container() {
        let root = tempfile::tempdir().expect("source directory");
        fs::create_dir(root.path().join("config")).expect("config directory");
        fs::write(root.path().join("config/user.toml"), "stream = true\n").expect("fixture");
        let output = root.path().join("backup.moc");
        let manifest = create(
            &output,
            root.path(),
            &[("config".to_owned(), PathBuf::from("config"))],
        )
        .expect("create MOC");
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(manifest.package_type, PackageType::Snapshot);
        assert_eq!(manifest.modules.len(), 1);
        assert_eq!(
            manifest.module_definitions,
            vec![ModuleDefinition {
                id: "config".to_owned(),
                path: "config".to_owned(),
                dependencies: Vec::new(),
                import_order: 10,
            }]
        );
        assert!(manifest.encryption.is_none());

        let extracted = tempfile::tempdir().expect("destination");
        let decoded =
            extract(&output, extracted.path(), ExtractionLimits::default()).expect("extract MOC");
        assert_eq!(decoded.modules, manifest.modules);
        assert_eq!(
            fs::read_to_string(extracted.path().join("config/user.toml")).expect("extracted file"),
            "stream = true\n"
        );
    }

    #[test]
    fn rejects_parent_path() {
        assert!(matches!(
            validate_relative(Path::new("../secret")),
            Err(MocError::UnsafePath(_))
        ));
    }

    #[test]
    fn inspects_encrypted_wrapper_metadata() {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir(root.path().join("private")).expect("private");
        fs::write(root.path().join("private/payload.enc"), "ciphertext").expect("payload");
        let output = root.path().join("private.moc");
        let encryption = EncryptionMetadata {
            profile: "momo-envelope-v1".to_owned(),
            payload_path: "private/payload.enc".to_owned(),
            associated_data: "momo-private-moc-v1".to_owned(),
        };
        create_with_encryption(
            &output,
            root.path(),
            &[("encrypted-container".to_owned(), PathBuf::from("private"))],
            Some(encryption.clone()),
        )
        .expect("create wrapper");
        assert_eq!(
            inspect(output).expect("inspect").encryption,
            Some(encryption)
        );
    }

    #[test]
    fn rejects_duplicate_manifest_paths_that_hide_an_unlisted_payload() {
        let root = tempfile::tempdir().expect("root");
        let output = root.path().join("malicious.moc");
        let declared = b"declared";
        let module = ModuleEntry {
            module: "config".to_owned(),
            path: "config/runtime.toml".to_owned(),
            size: declared.len() as u64,
            sha256: hex::encode(Sha256::digest(declared)),
        };
        let manifest = Manifest {
            format: FORMAT_NAME.to_owned(),
            format_version: FORMAT_VERSION,
            created_at: Utc::now(),
            package_type: PackageType::Snapshot,
            base_sequence: None,
            through_sequence: None,
            module_definitions: vec![ModuleDefinition {
                id: "config".to_owned(),
                path: "config".to_owned(),
                dependencies: Vec::new(),
                import_order: 10,
            }],
            modules: vec![module.clone(), module],
            deletions: Vec::new(),
            encryption: None,
        };
        write_raw_container(
            &output,
            &manifest,
            &[
                ("config/runtime.toml", declared),
                ("private/unlisted.txt", b"must not be accepted"),
            ],
        );

        let destination = tempfile::tempdir().expect("destination");
        assert!(matches!(
            extract(&output, destination.path(), ExtractionLimits::default()),
            Err(MocError::DuplicatePath(path)) if path == "config/runtime.toml"
        ));
        assert!(matches!(
            inspect(&output),
            Err(MocError::DuplicatePath(path)) if path == "config/runtime.toml"
        ));
    }

    #[test]
    fn v1_and_singular_module_ids_are_rejected() {
        let root = tempfile::tempdir().expect("root");
        let output = root.path().join("legacy.moc");
        let character = b"# Character";
        let legacy = Manifest {
            format: FORMAT_NAME.to_owned(),
            format_version: 1,
            created_at: Utc::now(),
            package_type: PackageType::Snapshot,
            base_sequence: None,
            through_sequence: None,
            module_definitions: Vec::new(),
            modules: vec![ModuleEntry {
                module: "character".to_owned(),
                path: "characters/card/character.md".to_owned(),
                size: character.len() as u64,
                sha256: hex::encode(Sha256::digest(character)),
            }],
            deletions: Vec::new(),
            encryption: None,
        };
        write_raw_container(
            &output,
            &legacy,
            &[("characters/card/character.md", character)],
        );

        let rejected = tempfile::tempdir().expect("rejected destination");
        assert!(matches!(
            extract(&output, rejected.path(), ExtractionLimits::default()),
            Err(MocError::UnsupportedFormatVersion { found: 1, .. })
        ));
        assert!(matches!(
            inspect(&output),
            Err(MocError::UnsupportedFormatVersion { found: 1, .. })
        ));

        fs::create_dir_all(root.path().join("characters")).expect("characters directory");
        fs::write(root.path().join("characters/card.md"), character).expect("character payload");
        assert!(matches!(
            create(
                root.path().join("singular.moc"),
                root.path(),
                &[("character".to_owned(), PathBuf::from("characters"))],
            ),
            Err(MocError::InvalidManifest(message))
                if message.contains("legacy module id character")
        ));
    }

    #[test]
    fn rejects_higher_versions_and_invalid_deletion_packages() {
        let manifest = Manifest {
            format: FORMAT_NAME.to_owned(),
            format_version: FORMAT_VERSION + 1,
            created_at: Utc::now(),
            package_type: PackageType::Snapshot,
            base_sequence: None,
            through_sequence: None,
            module_definitions: Vec::new(),
            modules: Vec::new(),
            deletions: Vec::new(),
            encryption: None,
        };
        assert!(matches!(
            validate_manifest(&manifest),
            Err(MocError::UnsupportedFormatVersion { .. })
        ));

        let invalid_deletion = Manifest {
            format_version: FORMAT_VERSION,
            package_type: PackageType::Deletion,
            base_sequence: Some(10),
            through_sequence: Some(11),
            ..manifest
        };
        assert!(matches!(
            validate_v2_manifest(&invalid_deletion),
            Err(MocError::InvalidManifest(_))
        ));
    }
}
