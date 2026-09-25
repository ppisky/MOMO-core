use super::*;

#[test]
fn defaults_include_general_and_product_prompts() {
    let spaces = PromptSpaces::in_memory();
    assert_eq!(
        spaces.get(PromptSpaceId::Assistant).content.trim(),
        "You are a helpful assistant."
    );
    assert!(
        spaces
            .get(PromptSpaceId::VisionFallback)
            .content
            .contains("visible facts")
    );
    assert_eq!(spaces.list().len(), PromptSpaceId::ALL.len());
}

#[test]
fn replacements_persist_and_reset_to_builtin() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("prompt-spaces.json");
    let spaces = PromptSpaces::load_or_default(&path).expect("registry");
    let replaced = spaces
        .replace(
            PromptSpaceId::Assistant,
            "You are a concise assistant.".to_owned(),
        )
        .expect("replace");
    assert_eq!(replaced.source, PromptSpaceSource::Override);

    let reloaded = PromptSpaces::load_or_default(&path).expect("reload");
    assert_eq!(
        reloaded.get(PromptSpaceId::Assistant).content,
        "You are a concise assistant."
    );
    let reset = reloaded.reset(PromptSpaceId::Assistant).expect("reset");
    assert_eq!(reset.source, PromptSpaceSource::Builtin);
    assert_eq!(reset.content.trim(), "You are a helpful assistant.");
}

#[test]
fn empty_and_unknown_prompt_spaces_are_rejected() {
    let spaces = PromptSpaces::in_memory();
    assert!(
        spaces
            .replace(PromptSpaceId::Assistant, " \n".to_owned())
            .is_err()
    );
    assert!("not_a_prompt".parse::<PromptSpaceId>().is_err());
}
