use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) struct QueryHit {
    pub(super) candidate: bool,
    pub(super) substantive: bool,
}

pub(super) fn query_hit(entry: &IndexEntry, id: &str, normalized_query: &str) -> QueryHit {
    let mut candidate = false;
    let mut substantive_terms = 0_usize;
    for term in entry.aliases.iter().chain(entry.tags.iter()) {
        let term = normalize(term);
        if term.is_empty() || !normalized_query.contains(&term) {
            continue;
        }
        candidate = true;
        if !is_generic_term(&term) {
            substantive_terms += 1;
        }
    }
    let id_term = normalize(id);
    if !id_term.is_empty() && normalized_query.contains(&id_term) {
        candidate = true;
        if !is_generic_term(&id_term) {
            substantive_terms += 1;
        }
    }
    QueryHit {
        candidate,
        substantive: substantive_terms > 0,
    }
}

pub(super) fn is_generic_term(term: &str) -> bool {
    term.chars().count() <= 1
        || matches!(
            term,
            "the"
                | "a"
                | "an"
                | "and"
                | "or"
                | "you"
                | "me"
                | "i"
                | "he"
                | "she"
                | "it"
                | "they"
                | "主角"
                | "角色"
                | "城市"
                | "魔法"
        )
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
                .is_some_and(|entry| access.can_read(&entry.kind))
        })
        .count()
}

pub(super) fn ranked_relation_ids(
    document: &MemoryDocument,
    by_id: &HashMap<String, &IndexEntry>,
    access: &AccessConfig,
    normalized_query: &str,
    hot_reference_ids: &HashSet<String>,
) -> Vec<String> {
    let mut ids = document
        .metadata
        .relations
        .values()
        .flatten()
        .filter_map(|id| {
            let entry = by_id.get(id)?;
            access
                .can_read(&entry.kind)
                .then(|| ((*id).clone(), *entry))
        })
        .collect::<Vec<_>>();
    ids.sort_by(|left, right| {
        let left_hot = hot_reference_ids.contains(&left.0);
        let right_hot = hot_reference_ids.contains(&right.0);
        let left_query = query_hit(left.1, &left.0, normalized_query).candidate;
        let right_query = query_hit(right.1, &right.0, normalized_query).candidate;
        right_query
            .cmp(&left_query)
            .then_with(|| right_hot.cmp(&left_hot))
            .then_with(|| left.0.cmp(&right.0))
    });
    ids.into_iter().map(|(id, _)| id).collect()
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
