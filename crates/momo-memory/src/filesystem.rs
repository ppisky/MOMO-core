use super::*;

pub(super) fn validate_patch_target(relative: &Path) -> Result<(), MemoryError> {
    validate_relative(relative)?;
    if relative.extension().and_then(|value| value.to_str()) != Some("md") {
        return Err(MemoryError::UnsafePath(relative.to_path_buf()));
    }
    let first = relative.components().next().and_then(component_name);
    if !matches!(
        first,
        Some("current" | "characters" | "relationships" | "events" | "world")
    ) {
        return Err(MemoryError::UnsafePath(relative.to_path_buf()));
    }
    Ok(())
}

pub(super) fn validate_document_location(
    relative: &Path,
    metadata: &Metadata,
) -> Result<(), MemoryError> {
    validate_metadata(metadata)?;
    let expected_kind = expected_kind_for_path(relative)
        .ok_or_else(|| MemoryError::UnsafePath(relative.to_path_buf()))?;
    if metadata.kind != expected_kind {
        return Err(MemoryError::InvalidPatch(format!(
            "document type {} does not match path {}",
            metadata.kind,
            relative.display()
        )));
    }
    Ok(())
}

pub(super) fn expected_kind_for_path(relative: &Path) -> Option<&'static str> {
    let mut components = relative.components();
    match components.next().and_then(component_name)? {
        "current" => Some("current"),
        "characters" => Some("character"),
        "relationships" => Some("relationship"),
        "events" => Some("event"),
        "world" => Some("world"),
        "archive" => match components.next().and_then(component_name)? {
            "character" => Some("character"),
            "relationship" => Some("relationship"),
            "event" => Some("event"),
            "world" => Some("world"),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn archive_path_for(relative: &Path, kind: &str) -> Result<PathBuf, MemoryError> {
    let active_directory = active_directory_for_kind(kind)
        .ok_or_else(|| MemoryError::InvalidPatch(format!("unsupported memory type: {kind}")))?;
    let suffix = relative
        .strip_prefix(active_directory)
        .map_err(|_| MemoryError::UnsafePath(relative.to_path_buf()))?;
    Ok(Path::new("archive").join(kind).join(suffix))
}

pub(super) fn active_path_for(relative: &Path, kind: &str) -> Result<PathBuf, MemoryError> {
    let active_directory = active_directory_for_kind(kind)
        .ok_or_else(|| MemoryError::InvalidPatch(format!("unsupported memory type: {kind}")))?;
    let suffix = relative
        .strip_prefix(Path::new("archive").join(kind))
        .map_err(|_| MemoryError::UnsafePath(relative.to_path_buf()))?;
    Ok(Path::new(active_directory).join(suffix))
}

pub(super) fn active_directory_for_kind(kind: &str) -> Option<&'static str> {
    ACTIVE_MEMORY_DIRECTORIES
        .iter()
        .find_map(|(directory, expected_kind)| (*expected_kind == kind).then_some(*directory))
}

pub(super) fn portable_path(path: &Path) -> String {
    path.components()
        .filter_map(component_name)
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn component_name<'a>(component: Component<'a>) -> Option<&'a str> {
    match component {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }
}

pub(super) fn collect_markdown_paths(
    root: &Path,
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), MemoryError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| MemoryError::UnsafePath(path.clone()))?;
        if file_type.is_symlink() {
            return Err(MemoryError::UnsafePath(relative.to_path_buf()));
        }
        if file_type.is_dir() {
            collect_markdown_paths(root, &path, output)?;
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("md")
        {
            output.push(relative.to_path_buf());
        }
    }
    Ok(())
}

pub(super) fn create_managed_directory(root: &Path, relative: &Path) -> Result<(), MemoryError> {
    validate_relative(relative)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(MemoryError::UnsafePath(relative.to_path_buf()));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(MemoryError::UnsafePath(relative.to_path_buf()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(super) fn regular_file_exists(path: &Path) -> Result<bool, MemoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(MemoryError::UnsafePath(path.to_path_buf()))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn validate_relative(path: &Path) -> Result<(), MemoryError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(MemoryError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

pub(super) fn collect_snapshot_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<(), MemoryError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(MemoryError::UnsafePath(path));
        }
        if file_type.is_dir() {
            collect_snapshot_files(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(MemoryError::UnsafePath(path));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| MemoryError::UnsafePath(path.clone()))?;
        validate_relative(relative)?;
        let key = relative
            .iter()
            .map(|component| component.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let content = fs::read_to_string(&path)?;
        files.insert(key, content);
    }
    Ok(())
}

pub(super) fn remove_workspace_files(directory: &Path) -> Result<(), MemoryError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(MemoryError::UnsafePath(path));
        }
        if file_type.is_dir() {
            remove_workspace_files(&path)?;
        } else if file_type.is_file() {
            fs::remove_file(&path)?;
        } else {
            return Err(MemoryError::UnsafePath(path));
        }
    }
    Ok(())
}

pub(super) fn write_if_missing(path: &Path, content: &str) -> Result<(), MemoryError> {
    if !regular_file_exists(path)? {
        atomic_write(path, content)?;
    }
    Ok(())
}

pub(super) fn atomic_write(path: &Path, content: &str) -> Result<(), MemoryError> {
    atomic_write_bytes(path, content.as_bytes())
}

pub(super) fn atomic_write_bytes(path: &Path, content: &[u8]) -> Result<(), MemoryError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(MemoryError::UnsafePath(parent.to_path_buf()));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}
