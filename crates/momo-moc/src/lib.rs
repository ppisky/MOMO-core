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

pub const CONTAINER_ENCODING: &str = "tar+zstandard";
pub const FORMAT_NAME: &str = "momo-container";
pub const FORMAT_VERSION: u32 = 3;
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;
pub const DEFAULT_MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub module_definitions: Vec<ModuleDefinition>,
    #[serde(default)]
    pub space_modules: Vec<SpaceModuleDefinition>,
    pub modules: Vec<ModuleEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModuleDefinition {
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    pub import_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpaceModuleDefinition {
    pub space_id: String,
    pub module: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EncryptionMetadata {
    pub profile: String,
    pub payload_path: String,
    pub associated_data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModuleEntry {
    pub module: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
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

pub fn create_from_definitions(
    output: impl AsRef<Path>,
    source_root: impl AsRef<Path>,
    modules: &[ModuleDefinition],
) -> Result<Manifest, MocError> {
    create_from_definitions_and_spaces_with_encryption(output, source_root, modules, &[], None)
}

pub fn create_from_definitions_and_spaces(
    output: impl AsRef<Path>,
    source_root: impl AsRef<Path>,
    modules: &[ModuleDefinition],
    space_modules: &[SpaceModuleDefinition],
) -> Result<Manifest, MocError> {
    create_from_definitions_and_spaces_with_encryption(
        output,
        source_root,
        modules,
        space_modules,
        None,
    )
}

pub fn create_with_encryption(
    output: impl AsRef<Path>,
    source_root: impl AsRef<Path>,
    modules: &[(String, PathBuf)],
    encryption: Option<EncryptionMetadata>,
) -> Result<Manifest, MocError> {
    let definitions = modules
        .iter()
        .map(|(module, source)| module_definition(module, source))
        .collect::<Result<Vec<_>, _>>()?;
    create_from_definitions_with_encryption(output, source_root, &definitions, encryption)
}

pub fn create_from_definitions_with_encryption(
    output: impl AsRef<Path>,
    source_root: impl AsRef<Path>,
    modules: &[ModuleDefinition],
    encryption: Option<EncryptionMetadata>,
) -> Result<Manifest, MocError> {
    create_from_definitions_and_spaces_with_encryption(
        output,
        source_root,
        modules,
        &[],
        encryption,
    )
}

pub fn create_from_definitions_and_spaces_with_encryption(
    output: impl AsRef<Path>,
    source_root: impl AsRef<Path>,
    modules: &[ModuleDefinition],
    space_modules: &[SpaceModuleDefinition],
    encryption: Option<EncryptionMetadata>,
) -> Result<Manifest, MocError> {
    let output = output.as_ref();
    let source_root = source_root.as_ref().canonicalize()?;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut module_definitions = modules.to_vec();
    let mut seen_modules = HashSet::new();

    for definition in modules {
        let module = &definition.id;
        let relative_source = Path::new(&definition.path);
        validate_relative(relative_source)?;
        validate_native_v3_module_id(module)?;
        if !seen_modules.insert(module.clone()) {
            return Err(MocError::InvalidManifest(format!(
                "duplicate module definition: {module}"
            )));
        }
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
                let space_id = space_for_path(space_modules, module, relative)?;
                push_entry(
                    module,
                    space_id,
                    relative,
                    item.path(),
                    &mut entries,
                    &mut seen,
                )?;
            }
        } else {
            let space_id = space_for_path(space_modules, module, relative_source)?;
            push_entry(
                module,
                space_id,
                relative_source,
                &source,
                &mut entries,
                &mut seen,
            )?;
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    module_definitions.sort_by_key(|module| (module.import_order, module.id.clone()));

    let manifest = Manifest {
        format: FORMAT_NAME.to_owned(),
        format_version: FORMAT_VERSION,
        created_at: Utc::now(),
        module_definitions,
        space_modules: space_modules.to_vec(),
        modules: entries,
        encryption,
    };
    validate_v3_manifest(&manifest)?;
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
        validate_v3_manifest(&manifest)?;
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
    space_id: Option<String>,
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
    let (size, sha256) = file_size_and_sha256(source)?;
    entries.push(ModuleEntry {
        module: module.to_owned(),
        space_id,
        path,
        size,
        sha256,
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
        let (actual_size, actual_hash) = file_size_and_sha256(path)?;
        if *size != expected.size || actual_size != expected.size || actual_hash != expected.sha256
        {
            return Err(MocError::Integrity(expected.path.clone()));
        }
    }
    Ok(())
}

fn file_size_and_sha256(path: &Path) -> Result<(u64, String), MocError> {
    let mut file = File::open(path)?;
    let expected_size = file.metadata()?.len();
    let mut actual_size = 0_u64;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        actual_size = actual_size
            .checked_add(u64::try_from(read).map_err(|_| MocError::LimitExceeded)?)
            .ok_or(MocError::LimitExceeded)?;
        digest.update(&buffer[..read]);
    }
    if actual_size != expected_size {
        return Err(MocError::Integrity(path.display().to_string()));
    }
    Ok((actual_size, hex::encode(digest.finalize())))
}

fn validate_manifest(manifest: &Manifest) -> Result<(), MocError> {
    if manifest.format_version == FORMAT_VERSION {
        validate_v3_manifest(manifest)
    } else {
        Err(MocError::UnsupportedFormatVersion {
            found: manifest.format_version,
            supported: FORMAT_VERSION,
        })
    }
}

fn validate_v3_manifest(manifest: &Manifest) -> Result<(), MocError> {
    validate_manifest_paths(manifest)?;
    let mut definitions = HashMap::new();
    for definition in &manifest.module_definitions {
        validate_native_v3_module_id(&definition.id)?;
        validate_manifest_path(&definition.path)?;
        if let Some((path, dependencies, import_order)) = known_module_layout(&definition.id)
            && (definition.path != path
                || definition.dependencies != dependencies
                || definition.import_order != import_order)
        {
            return Err(MocError::InvalidManifest(format!(
                "module {} does not use its canonical v3 layout",
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
    let mut declared_spaces = HashMap::new();
    let mut target_spaces = HashSet::new();
    for space in &manifest.space_modules {
        let space_id = uuid::Uuid::parse_str(&space.space_id).map_err(|_| {
            MocError::InvalidManifest(format!("space id is not a UUID: {}", space.space_id))
        })?;
        if !is_space_owned_module(&space.module) {
            return Err(MocError::InvalidManifest(format!(
                "module {} cannot be declared as Space-owned",
                space.module
            )));
        }
        if !definitions.contains_key(&space.module) {
            return Err(MocError::InvalidManifest(format!(
                "Space module {} has no module definition",
                space.module
            )));
        }
        let expected = format!("{}/spaces/{space_id}", space.module);
        if space.path != expected {
            return Err(MocError::InvalidManifest(format!(
                "Space module {} must use {}",
                space.module, expected
            )));
        }
        validate_manifest_path(&space.path)?;
        if declared_spaces
            .insert((space.module.clone(), space.space_id.clone()), space)
            .is_some()
            || !target_spaces.insert(space.path.clone())
        {
            return Err(MocError::InvalidManifest(format!(
                "duplicate Space module declaration: {} {}",
                space.module, space.space_id
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
        if is_space_owned_module(&entry.module) {
            let space_id = entry.space_id.as_ref().ok_or_else(|| {
                MocError::InvalidManifest(format!(
                    "Space-owned payload {} has no space_id",
                    entry.path
                ))
            })?;
            let space = declared_spaces
                .get(&(entry.module.clone(), space_id.clone()))
                .ok_or_else(|| {
                    MocError::InvalidManifest(format!(
                        "payload {} references undeclared Space {}",
                        entry.path, space_id
                    ))
                })?;
            if entry.path != space.path
                && !entry
                    .path
                    .strip_prefix(&space.path)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            {
                return Err(MocError::InvalidManifest(format!(
                    "payload {} is outside declared Space {}",
                    entry.path, space_id
                )));
            }
        } else if entry.space_id.is_some() {
            return Err(MocError::InvalidManifest(format!(
                "non-Space payload {} declares a space_id",
                entry.path
            )));
        }
    }
    Ok(())
}

fn validate_native_v3_module_id(module: &str) -> Result<(), MocError> {
    validate_module_id(module)?;
    if matches!(module, "character" | "conversation") {
        return Err(MocError::InvalidManifest(format!(
            "legacy module id {module} is not valid in native v3"
        )));
    }
    Ok(())
}

fn is_space_owned_module(module: &str) -> bool {
    matches!(
        module,
        "characters" | "conversations" | "memory" | "semantic_graph"
    )
}

fn space_for_path(
    spaces: &[SpaceModuleDefinition],
    module: &str,
    path: &Path,
) -> Result<Option<String>, MocError> {
    let path = normalized_path(path)?;
    let matching = spaces
        .iter()
        .filter(|space| {
            space.module == module
                && (path == space.path
                    || path
                        .strip_prefix(&space.path)
                        .is_some_and(|suffix| suffix.starts_with('/')))
        })
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        return Err(MocError::InvalidManifest(format!(
            "payload {path} matches multiple Space declarations"
        )));
    }
    if is_space_owned_module(module) && matching.is_empty() {
        return Err(MocError::InvalidManifest(format!(
            "Space-owned payload {path} is not declared"
        )));
    }
    Ok(matching.first().map(|space| space.space_id.clone()))
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
#[path = "../tests/unit/lib.rs"]
mod tests;
