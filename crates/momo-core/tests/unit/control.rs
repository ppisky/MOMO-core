use super::*;

#[test]
fn clear_memory_requires_an_explicit_module() {
    let request = MomoControlRequest {
        schema: MOMO_CONTROL_SCHEMA.to_owned(),
        request_id: "control-1".to_owned(),
        actor_space_id: Uuid::now_v7(),
        action: MomoControlAction::ClearMemory {
            target_space_id: Uuid::now_v7(),
            memory: false,
            semantic_graph: false,
        },
    };
    assert!(request.validate().is_err());
}

#[test]
fn parses_the_frozen_control_request() {
    let request: MomoControlRequest = serde_json::from_str(include_str!(
        "../../../../contracts/1.0/control_request.json"
    ))
    .expect("control fixture");
    request.validate().expect("valid control fixture");
    assert_eq!(request.schema, MOMO_CONTROL_SCHEMA);
}
