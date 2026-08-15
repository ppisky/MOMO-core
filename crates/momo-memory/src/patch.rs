use super::*;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PatchDocument {
    pub(super) patches: Vec<FilePatch>,
}

pub(super) fn parse_patch_document(yaml: &str) -> Result<PatchDocument, MemoryError> {
    let quoted_content_repaired = escape_quoted_content_backslashes(yaml);
    match yaml_serde::from_str(&quoted_content_repaired) {
        Ok(patch) => Ok(patch),
        Err(original_error) => {
            let repaired = escape_unknown_double_quoted_yaml_escapes(&quoted_content_repaired);
            if repaired == quoted_content_repaired {
                return Err(original_error.into());
            }
            yaml_serde::from_str(&repaired).map_err(Into::into)
        }
    }
}

pub(super) fn escape_quoted_content_backslashes(value: &str) -> String {
    value
        .split_inclusive('\n')
        .map(|line| {
            let Some(marker) = line.find("content: \"") else {
                return line.to_owned();
            };
            let quote_start = marker + "content: ".len();
            let mut output = String::with_capacity(line.len());
            output.push_str(&line[..=quote_start]);
            let mut chars = line[quote_start + 1..].chars().peekable();
            while let Some(character) = chars.next() {
                if character != '\\' {
                    output.push(character);
                    continue;
                }
                output.push('\\');
                let next = chars.next();
                if !matches!(next, Some('\\' | '"')) {
                    output.push('\\');
                }
                if let Some(next) = next {
                    output.push(next);
                }
            }
            output
        })
        .collect()
}

pub(super) fn escape_unknown_double_quoted_yaml_escapes(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut in_double_quote = false;
    while let Some(character) = chars.next() {
        if character == '"' {
            in_double_quote = !in_double_quote;
            output.push(character);
            continue;
        }
        if in_double_quote && character == '\\' {
            let next = chars.next();
            let valid = matches!(
                next,
                Some(
                    '0' | 'a'
                        | 'b'
                        | 't'
                        | 'n'
                        | 'v'
                        | 'f'
                        | 'r'
                        | 'e'
                        | ' '
                        | '"'
                        | '/'
                        | '\\'
                        | 'N'
                        | '_'
                        | 'L'
                        | 'P'
                        | 'x'
                        | 'u'
                        | 'U'
                        | '\n'
                        | '\r'
                        | '\t'
                )
            );
            output.push('\\');
            if !valid {
                output.push('\\');
            }
            if let Some(next) = next {
                output.push(next);
            }
            continue;
        }
        output.push(character);
    }
    output
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FilePatch {
    pub(super) target_file: String,
    pub(super) operations: Vec<PatchOperation>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PatchOperation {
    Append {
        section: String,
        content: String,
    },
    Replace {
        section: String,
        content: String,
    },
    Create {
        frontmatter: CreateMetadata,
        content: String,
    },
    UpdateFrontmatter {
        fields: MetadataUpdate,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateMetadata {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    importance: f64,
    weight: f64,
    decay_at: i64,
    #[serde(default)]
    relations: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    injection_scope: Option<String>,
    #[serde(default)]
    injection_conversation_id: Option<String>,
    #[serde(default)]
    injection_character_id: Option<String>,
    status: String,
}

impl CreateMetadata {
    pub(super) fn into_metadata(self) -> Metadata {
        Metadata {
            id: self.id,
            kind: self.kind,
            importance: Some(self.importance),
            weight: Some(self.weight),
            touch_at: 0,
            decay_at: Some(self.decay_at),
            archived_at: None,
            relations: self.relations,
            tags: self.tags,
            aliases: self.aliases,
            injection_scope: self.injection_scope,
            injection_conversation_id: self.injection_conversation_id,
            injection_character_id: self.injection_character_id,
            status: self.status,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MetadataUpdate {
    #[serde(default)]
    importance: Option<f64>,
    #[serde(default)]
    weight: Option<f64>,
    #[serde(default)]
    decay_at: Option<i64>,
    #[serde(default)]
    relations: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    aliases: Option<Vec<String>>,
    #[serde(default)]
    injection_scope: Option<String>,
    #[serde(default)]
    injection_conversation_id: Option<String>,
    #[serde(default)]
    injection_character_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

impl MetadataUpdate {
    fn is_empty(&self) -> bool {
        self.importance.is_none()
            && self.weight.is_none()
            && self.decay_at.is_none()
            && self.relations.is_none()
            && self.tags.is_none()
            && self.aliases.is_none()
            && self.injection_scope.is_none()
            && self.injection_conversation_id.is_none()
            && self.injection_character_id.is_none()
            && self.status.is_none()
    }
}

pub(super) fn apply_operation(
    document: &mut MemoryDocument,
    operation: PatchOperation,
) -> Result<(), MemoryError> {
    match operation {
        PatchOperation::Append { section, content } => {
            document.body = mutate_section(&document.body, &section, &content, false)?;
        }
        PatchOperation::Replace { section, content } => {
            document.body = mutate_section(&document.body, &section, &content, true)?;
        }
        PatchOperation::UpdateFrontmatter { fields } => {
            if fields.is_empty() {
                return Err(MemoryError::InvalidPatch(
                    "update_frontmatter fields is empty".to_owned(),
                ));
            }
            if let Some(value) = fields.importance {
                document.metadata.importance = Some(value);
            }
            if let Some(value) = fields.weight {
                document.metadata.weight = Some(value);
            }
            if let Some(value) = fields.decay_at {
                document.metadata.decay_at = Some(value);
            }
            if let Some(value) = fields.relations {
                document.metadata.relations = value;
            }
            if let Some(value) = fields.tags {
                document.metadata.tags = value;
            }
            if let Some(value) = fields.aliases {
                document.metadata.aliases = value;
            }
            if let Some(value) = fields.injection_scope {
                document.metadata.injection_scope = Some(value);
            }
            if let Some(value) = fields.injection_conversation_id {
                document.metadata.injection_conversation_id = Some(value);
            }
            if let Some(value) = fields.injection_character_id {
                document.metadata.injection_character_id = Some(value);
            }
            if let Some(value) = fields.status {
                document.metadata.status = value;
            }
        }
        PatchOperation::Create { .. } => {
            return Err(MemoryError::InvalidPatch(
                "create cannot modify an existing document".to_owned(),
            ));
        }
    }
    validate_metadata(&document.metadata)
}

pub(super) fn mutate_section(
    body: &str,
    section: &str,
    content: &str,
    replace: bool,
) -> Result<String, MemoryError> {
    let heading = format!("## {}", section.trim());
    let start = body
        .lines()
        .scan(0_usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((current, line))
        })
        .find(|(_, line)| *line == heading)
        .map(|(offset, _)| offset);
    let Some(start) = start else {
        return Err(MemoryError::MissingSection(section.to_owned()));
    };
    let content_start = start + heading.len();
    let next_heading = body[content_start..]
        .find("\n## ")
        .map_or(body.len(), |offset| content_start + offset + 1);
    let mut output = String::new();
    output.push_str(&body[..content_start]);
    output.push('\n');
    if !replace {
        output.push_str(body[content_start..next_heading].trim_matches('\n'));
        if !output.ends_with('\n') {
            output.push('\n');
        }
    }
    output.push_str(content.trim());
    output.push('\n');
    output.push_str(&body[next_heading..]);
    Ok(output)
}

pub(super) fn compare_documents(
    (left_path, left): &(String, MemoryDocument),
    (right_path, right): &(String, MemoryDocument),
) -> Ordering {
    right
        .metadata
        .weight
        .unwrap_or_default()
        .total_cmp(&left.metadata.weight.unwrap_or_default())
        .then_with(|| {
            right
                .metadata
                .importance
                .unwrap_or_default()
                .total_cmp(&left.metadata.importance.unwrap_or_default())
        })
        .then_with(|| right.metadata.touch_at.cmp(&left.metadata.touch_at))
        .then_with(|| left.metadata.id.cmp(&right.metadata.id))
        .then_with(|| left_path.cmp(right_path))
}

pub(super) fn validate_metadata(metadata: &Metadata) -> Result<(), MemoryError> {
    if metadata.id.trim().is_empty()
        || !matches!(
            metadata.kind.as_str(),
            "current" | "character" | "relationship" | "event" | "world"
        )
        || !matches!(metadata.status.as_str(), "active" | "archived")
    {
        return Err(MemoryError::InvalidFrontmatter);
    }
    for value in [metadata.importance, metadata.weight].into_iter().flatten() {
        if !(0.0..=1.0).contains(&value) {
            return Err(MemoryError::InvalidFrontmatter);
        }
    }
    if let Some(scope) = metadata.injection_scope.as_deref() {
        match scope {
            "conversation" => {
                if metadata
                    .injection_conversation_id
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err(MemoryError::InvalidFrontmatter);
                }
            }
            "character" => {
                if metadata
                    .injection_character_id
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err(MemoryError::InvalidFrontmatter);
                }
            }
            "account" => {}
            _ => return Err(MemoryError::InvalidFrontmatter),
        }
    }
    if metadata.kind == "current" {
        if metadata.importance.is_some()
            || metadata.weight.is_some()
            || metadata.decay_at.is_some()
            || metadata.archived_at.is_some()
            || metadata.injection_scope.is_some()
            || metadata.injection_conversation_id.is_some()
            || metadata.injection_character_id.is_some()
            || metadata.status != "active"
        {
            return Err(MemoryError::InvalidFrontmatter);
        }
    } else if metadata.importance.is_none()
        || metadata.weight.is_none()
        || metadata.decay_at.is_none()
        || (metadata.status == "active" && metadata.archived_at.is_some())
    {
        return Err(MemoryError::InvalidFrontmatter);
    }
    Ok(())
}
