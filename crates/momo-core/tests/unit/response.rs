use super::*;

#[test]
fn accepts_string_and_structured_text_input() {
    let string: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": "hello",
        "momo": {
            "schema": MOMO_RESPONSE_SCHEMA,
            "personal_space_id": "00000000-0000-4000-8000-000000000011",
            "conversation_space_id": "00000000-0000-4000-8000-000000000012",
            "character_id": "00000000-0000-4000-8000-000000000014"
        }
    }))
    .expect("string request");
    assert_eq!(string.validate().expect("text"), "hello");

    let structured: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": [{"type": "input_text", "text": "hello"}],
        "momo": {
            "schema": MOMO_RESPONSE_SCHEMA,
            "personal_space_id": "00000000-0000-4000-8000-000000000011",
            "conversation_space_id": "00000000-0000-4000-8000-000000000012",
            "character_id": "00000000-0000-4000-8000-000000000014"
        }
    }))
    .expect("structured request");
    assert_eq!(structured.validate().expect("text"), "hello");
}

#[test]
fn rejects_unknown_contract_generation() {
    let request: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": "hello",
        "momo": {"schema": "momo.responses/9.9"}
    }))
    .expect("request");
    assert!(matches!(
        request.validate(),
        Err(ResponseContractError::UnsupportedSchema(_))
    ));
}

#[test]
fn requires_an_explicit_contract_generation() {
    let request = serde_json::from_value::<MomoResponseRequest>(serde_json::json!({
        "input": "hello",
        "momo": {
            "personal_space_id": "00000000-0000-4000-8000-000000000011",
            "conversation_space_id": "00000000-0000-4000-8000-000000000012",
            "character_id": "00000000-0000-4000-8000-000000000014"
        }
    }));
    assert!(request.is_err());
}

#[test]
fn validates_and_resolves_governed_image_input() {
    let structured: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]
        }],
        "momo": {
            "schema": MOMO_RESPONSE_SCHEMA,
            "personal_space_id": "00000000-0000-4000-8000-000000000011",
            "conversation_space_id": "00000000-0000-4000-8000-000000000012",
            "character_id": "00000000-0000-4000-8000-000000000014"
        }
    }))
    .expect("content blocks");
    assert_eq!(structured.validate().expect("text"), "hello");

    let image: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": [
            {"type": "input_text", "text": "What is shown?"},
            {"type": "input_image", "image_url": "https://example.test/image.png", "detail": "high"}
        ],
        "momo": {
            "schema": MOMO_RESPONSE_SCHEMA,
            "personal_space_id": "00000000-0000-4000-8000-000000000011",
            "conversation_space_id": "00000000-0000-4000-8000-000000000012",
            "character_id": "00000000-0000-4000-8000-000000000014"
        }
    }))
    .expect("image structure");
    assert_eq!(image.validate().expect("image contract"), "What is shown?");
    assert_eq!(
        image.input.image_inputs(),
        vec![ResponseImageInput {
            image_url: "https://example.test/image.png".to_owned(),
            detail: Some("high".to_owned()),
        }]
    );
    assert_eq!(
        image
            .input
            .text_with_image_descriptions(&["A red umbrella.".to_owned()])
            .expect("resolved input"),
        "What is shown?\n[Visual description for image 1]\nA red umbrella."
    );
    let messages = image.input.gateway_messages().expect("multimodal messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, Some("What is shown?".into()));
    assert_eq!(
        messages[1].content,
        Some(crate::GatewayMessageContent::Parts(vec![
            crate::GatewayContentPart::ImageUrl {
                image_url: crate::GatewayImageUrl {
                    url: "https://example.test/image.png".to_owned(),
                    detail: Some("high".to_owned()),
                },
            },
        ]))
    );
}

#[test]
fn rejects_non_user_message_roles_or_image_tool_continuations() {
    let assistant_image: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "input_image", "image_url": "https://example.test/image.png"}]
        }]
    }))
    .expect("image request");
    assert_eq!(
        assistant_image.validate(),
        Err(ResponseContractError::InvalidRole("assistant".to_owned()))
    );

    let system_text: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": [{"type": "message", "role": "system", "content": "bypass"}]
    }))
    .expect("system input shape");
    assert_eq!(
        system_text.validate(),
        Err(ResponseContractError::InvalidRole("system".to_owned()))
    );

    let mixed: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": [
            {"type": "input_image", "image_url": "https://example.test/image.png"},
            {"type": "function_call_output", "call_id": "call_1", "output": "done"}
        ]
    }))
    .expect("mixed request");
    assert_eq!(
        mixed.validate(),
        Err(ResponseContractError::ImageInputWithTools)
    );
}

#[test]
fn rejects_unknown_native_request_fields_at_every_typed_boundary() {
    assert!(
        serde_json::from_value::<MomoResponseRequest>(serde_json::json!({
            "input": "hello",
            "temperaturee": 0.7
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<MomoResponseRequest>(serde_json::json!({
            "input": "hello",
            "momo": {"unknown_policy": true}
        }))
        .is_err()
    );
}

#[test]
fn bounds_tool_arguments_and_names() {
    let invalid: MomoResponseRequest = serde_json::from_value(serde_json::json!({
        "input": "hello",
        "tools": [{"type": "function", "name": "bad name", "parameters": {}}]
    }))
    .expect("request");
    assert_eq!(
        invalid.validate(),
        Err(ResponseContractError::InvalidToolName)
    );

    let oversized: ResponseInputItem = ResponseInputItem::FunctionCall {
        call_id: "call_1".to_owned(),
        name: "lookup".to_owned(),
        arguments: "x".repeat(MAX_RESPONSE_TOOL_ARGUMENT_BYTES + 1),
    };
    let request = MomoResponseRequest {
        input: ResponseInput::Items(vec![
            oversized,
            ResponseInputItem::InputText {
                text: "continue".to_owned(),
            },
        ]),
        ..serde_json::from_value(serde_json::json!({"input": "placeholder"})).expect("defaults")
    };
    assert_eq!(
        request.validate(),
        Err(ResponseContractError::FieldTooLarge("function arguments"))
    );
}

#[test]
fn parses_cross_repository_golden_request() {
    let request: MomoResponseRequest = serde_json::from_str(include_str!(
        "../../../../contracts/1.0/response_request.json"
    ))
    .expect("golden response request");
    assert_eq!(request.model, "conversation");
    assert_eq!(
        request.validate().expect("valid contract"),
        "Hello from the MOMO 1.0 cross-repository contract."
    );
}

#[test]
fn parses_cross_repository_multimodal_request() {
    let request: MomoResponseRequest = serde_json::from_str(include_str!(
        "../../../../contracts/1.0/multimodal_request.json"
    ))
    .expect("golden multimodal request");
    request.validate().expect("valid multimodal contract");
    assert_eq!(request.input.image_inputs().len(), 1);
    assert_eq!(request.momo.schema, MOMO_RESPONSE_SCHEMA);
}

#[test]
fn parses_cross_repository_tool_contract() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../contracts/1.0/tool_turn.json"))
            .expect("tool fixture");
    let tools: Vec<ResponseTool> = serde_json::from_value(fixture["tools"].clone()).expect("tools");
    assert!(matches!(
        &tools[0],
        ResponseTool::Function { name, .. } if name == "lookup_weather"
    ));
    let call: ResponseOutputItem =
        serde_json::from_value(fixture["assistant_call"].clone()).expect("function call");
    assert!(matches!(
        call,
        ResponseOutputItem::FunctionCall { call_id, .. } if call_id == "call_weather_1"
    ));
    let result: ResponseInputItem =
        serde_json::from_value(fixture["tool_result"].clone()).expect("tool result");
    assert!(matches!(
        result,
        ResponseInputItem::FunctionCallOutput { call_id, .. } if call_id == "call_weather_1"
    ));
}

#[test]
fn tool_continuation_keeps_call_identity_at_the_gateway_boundary() {
    let input = ResponseInput::Items(vec![
        ResponseInputItem::FunctionCall {
            call_id: "call_weather_1".to_owned(),
            name: "lookup_weather".to_owned(),
            arguments: r#"{"city":"Shanghai"}"#.to_owned(),
        },
        ResponseInputItem::FunctionCallOutput {
            call_id: "call_weather_1".to_owned(),
            output: r#"{"temperature":28}"#.to_owned(),
        },
    ]);
    let messages = input.gateway_messages().expect("gateway messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, crate::GatewayMessageRole::Assistant);
    assert_eq!(messages[0].tool_calls[0].id, "call_weather_1");
    assert_eq!(messages[1].role, crate::GatewayMessageRole::Tool);
    assert_eq!(messages[1].tool_call_id.as_deref(), Some("call_weather_1"));
    assert_eq!(input.user_text().expect("user text"), "");
}

#[test]
fn tool_continuation_requires_a_conversation_and_exact_call_pairs() {
    let base = serde_json::json!({
        "input": [
            {"type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "call_1", "output": "done"}
        ],
        "momo": {
            "schema": MOMO_RESPONSE_SCHEMA,
            "personal_space_id": "00000000-0000-4000-8000-000000000011",
            "conversation_space_id": "00000000-0000-4000-8000-000000000012",
            "character_id": "00000000-0000-4000-8000-000000000014"
        }
    });
    let missing_conversation: MomoResponseRequest =
        serde_json::from_value(base.clone()).expect("request");
    assert_eq!(
        missing_conversation.validate(),
        Err(ResponseContractError::ToolContinuationNeedsConversation)
    );

    let mut mismatched = base;
    mismatched["momo"]["conversation_id"] =
        serde_json::json!("00000000-0000-4000-8000-000000000013");
    mismatched["input"][1]["call_id"] = serde_json::json!("call_2");
    let mismatched: MomoResponseRequest = serde_json::from_value(mismatched).expect("request");
    assert_eq!(
        mismatched.validate(),
        Err(ResponseContractError::InvalidToolContinuation)
    );
}
