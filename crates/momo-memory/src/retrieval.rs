use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) struct QueryHit {
    pub(super) candidate: bool,
    pub(super) substantive: bool,
}

pub(super) fn query_hit(entry: &IndexEntry, id: &str, normalized_query: &str) -> QueryHit {
    let mut candidate = false;
    let mut matched_terms = HashSet::new();
    let query_terms = searchable_terms(normalized_query);
    // Aliases are human-readable phrases, so matching their individual words
    // supports queries such as "silver astrolabe" against a longer title.
    for term in &entry.aliases {
        let term = normalize(term);
        if term.is_empty() {
            continue;
        }
        if normalized_query.contains(&term) {
            candidate = true;
            for token in searchable_terms(&term) {
                if !is_generic_term(&token) {
                    matched_terms.insert(token);
                }
            }
            continue;
        }
        for token in searchable_terms(&term) {
            if query_terms.contains(&token) && !is_generic_term(&token) {
                candidate = true;
                matched_terms.insert(token);
            }
        }
    }
    // Tags and IDs are structured identifiers. Splitting `unique_00049` and
    // matching its shared `unique` prefix would select every `unique_*` item.
    for term in &entry.tags {
        let term = normalize(term);
        if !term.is_empty() && contains_structured_term(normalized_query, &term) {
            candidate = true;
            for token in searchable_terms(&term) {
                if !is_generic_term(&token) {
                    matched_terms.insert(token);
                }
            }
        }
    }
    for identifier in &entry.body_identifiers {
        if query_terms.contains(identifier) {
            candidate = true;
            matched_terms.insert(identifier.clone());
        }
    }
    // Generated memories do not always have useful aliases or tags. Index
    // meaningful body terms as a deterministic fallback so a document that
    // actually contains the queried entities remains reachable. Exact token
    // membership avoids the structured-prefix fan-out fixed above.
    for term in &entry.body_terms {
        if query_terms.contains(term) {
            candidate = true;
            matched_terms.insert(term.clone());
        }
    }
    let id_term = normalize(id);
    if !id_term.is_empty() && contains_structured_term(normalized_query, &id_term) {
        candidate = true;
        if !is_generic_term(&id_term) {
            matched_terms.insert(id_term);
        }
    }
    QueryHit {
        candidate,
        substantive: !matched_terms.is_empty(),
    }
}

fn searchable_terms(value: &str) -> HashSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .flat_map(segment_terms)
        .collect()
}

fn contains_structured_term(query: &str, term: &str) -> bool {
    // Scripts without explicit word separators still need natural substring
    // recall. ASCII identifiers and tags, on the other hand, must respect
    // identifier boundaries so S17 cannot select S170.
    if !term.chars().any(is_structured_identifier_character) {
        return query.contains(term);
    }
    query.match_indices(term).any(|(start, matched)| {
        let end = start + matched.len();
        let left_is_boundary = query[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_structured_identifier_character(character));
        let right_is_boundary = query[end..]
            .chars()
            .next()
            .is_none_or(|character| !is_structured_identifier_character(character));
        left_is_boundary && right_is_boundary
    })
}

fn is_structured_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn segment_terms(segment: &str) -> Vec<String> {
    let mut runs = Vec::<String>::new();
    for character in segment.chars() {
        let starts_new_run = runs.last().is_some_and(|run| {
            run.chars()
                .next()
                .is_some_and(|first| first.is_ascii() != character.is_ascii())
        });
        if starts_new_run || runs.is_empty() {
            runs.push(String::new());
        }
        runs.last_mut().expect("run exists").push(character);
    }
    runs.into_iter()
        .flat_map(|run| {
            let characters = run.chars().collect::<Vec<_>>();
            if characters.len() > 2 && characters.iter().all(|character| !character.is_ascii()) {
                characters
                    .windows(2)
                    .map(|pair| pair.iter().collect())
                    .collect()
            } else {
                vec![run]
            }
        })
        .collect()
}

pub(super) fn body_identifiers(body: &str) -> Vec<String> {
    let mut identifiers = searchable_terms(&normalize(body))
        .into_iter()
        .filter(|term| {
            term.chars().any(|c| c.is_ascii_alphabetic())
                && term.chars().any(|c| c.is_ascii_digit())
        })
        .collect::<Vec<_>>();
    identifiers.sort();
    identifiers
}

pub(super) fn body_terms(body: &str) -> Vec<String> {
    let mut terms = searchable_terms(&normalize(body))
        .into_iter()
        .filter(|term| !is_generic_term(term))
        .collect::<Vec<_>>();
    terms.sort();
    terms
}

pub(super) fn is_generic_term(term: &str) -> bool {
    // Natural-language stop-word lists make retrieval behavior depend on the
    // implementation's preferred language. Only reject structurally empty
    // one-character terms; relevance remains data-driven for every script.
    term.chars().count() <= 1
}

pub(super) fn explicit_memory_references(text: &str) -> HashSet<String> {
    let mut references = HashSet::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("]]") else {
            break;
        };
        let value = after_start[..end].trim();
        if !value.is_empty()
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })
        {
            references.insert(value.to_owned());
        }
        rest = &after_start[end + 2..];
    }
    references
}

pub(super) fn relation_degree(
    document: &MemoryDocument,
    by_id: &HashMap<String, &IndexEntry>,
    access: &AccessConfig,
) -> usize {
    document
        .metadata
        .relations
        .values()
        .flatten()
        .filter(|id| {
            by_id
                .get(*id)
                .is_some_and(|entry| entry.is_active() && access.can_read(&entry.kind))
        })
        .count()
}

impl IndexEntry {
    fn is_active(&self) -> bool {
        self.status.as_deref().map_or_else(
            || !Path::new(&self.path).starts_with("archive"),
            |status| status == "active",
        )
    }
}

pub(super) fn normalize(value: &str) -> String {
    value.nfkc().collect::<String>().trim().to_lowercase()
}

pub(super) fn markdown_prefix_within_budget(
    text: &str,
    max_tokens: usize,
    counter: &impl TokenCounter,
) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    if counter.count(text) <= max_tokens {
        return text.to_owned();
    }
    let mut output = String::new();
    for paragraph in text.split_inclusive("\n\n") {
        let previous_len = output.len();
        output.push_str(paragraph);
        if counter.count(&output) > max_tokens {
            output.truncate(previous_len);
            break;
        }
    }
    output
}
