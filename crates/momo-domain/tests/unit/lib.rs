use super::*;

#[test]
fn generated_ids_are_uuid_v7() {
    let id = new_id();
    assert_eq!(id.get_version_num(), 7);
}

#[test]
fn roles_use_wire_values() {
    assert_eq!(MessageRole::Assistant.as_str(), "assistant");
    assert_eq!(MessageRole::try_from("user"), Ok(MessageRole::User));
}
