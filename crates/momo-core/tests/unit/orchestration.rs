use super::*;

#[test]
fn maintenance_rejects_mutated_opaque_identifiers() {
    let source = r#"Meet at Glass-Archive-f41e9 with Cobalt-Key-dae44."#;
    assert!(validate_generated_opaque_identifiers(source, source).is_ok());
    assert!(
        validate_generated_opaque_identifiers(
            source,
            "Store it at Glass-Archive-f41e with Cobalt-Key-dae44."
        )
        .is_err()
    );
}

#[test]
fn ddm_previous_bands_reset_when_the_profile_revision_changes() {
    let state = json!({
        "profile_revision": 7,
        "bands": {"protect_companion": "dominant"}
    });
    assert_eq!(
        compatible_ddm_bands(Some(&state), Some(7)),
        json!({"protect_companion": "dominant"})
    );
    assert_eq!(compatible_ddm_bands(Some(&state), Some(8)), json!({}));
    assert_eq!(compatible_ddm_bands(None, Some(7)), json!({}));
}

#[test]
fn maintenance_noop_detection_accepts_only_an_empty_patch() {
    assert!(memory_patch_is_noop("patches: []"));
    assert!(memory_patch_is_noop("patches:\n  []\n"));
    assert!(!memory_patch_is_noop(
        "patches:\n  - target_file: events/example.md"
    ));
}

#[test]
fn maintenance_accepts_only_a_normal_model_finish() {
    assert_eq!(
        maintenance_finish_error(&json!({"finish_reason": "stop"})),
        None
    );
    assert_eq!(
        maintenance_finish_error(&json!({"finish_reason": "length"})).as_deref(),
        Some("maintenance model did not finish normally (finish_reason=length)")
    );
    assert_eq!(
        maintenance_finish_error(&json!({})).as_deref(),
        Some("maintenance model did not finish normally (finish_reason=missing)")
    );
}

#[test]
fn mo_state_shadow_and_degraded_results_never_enter_the_prompt() {
    let healthy = json!({"context": "[STATE_CONTEXT]", "audit": {"degraded": false}});
    assert_eq!(
        state_context_for_prompt(&healthy, true, MoStateInjectionMode::Active),
        ("[STATE_CONTEXT]".to_owned(), "active")
    );
    assert_eq!(
        state_context_for_prompt(&healthy, true, MoStateInjectionMode::Shadow),
        (String::new(), "shadow")
    );
    let degraded = json!({"context": "unsafe", "audit": {"degraded": true}});
    assert_eq!(
        state_context_for_prompt(&degraded, true, MoStateInjectionMode::Active),
        (String::new(), "suppressed_degraded")
    );
}

#[test]
fn retrieval_audit_exposes_ids_and_sources_without_memory_bodies() {
    let audit = retrieval_audit(
        &[json!({"id": "memory-1", "body": "private text",
        "estimated_tokens": 12, "memory_space": {"id": "space-1", "label": "personal"}})],
        true,
        "ok",
    );
    assert_eq!(audit["entries"][0]["id"], "memory-1");
    assert_eq!(audit["entries"][0]["space_id"], "space-1");
    assert_eq!(audit["status"], "ok");
    assert!(!audit.to_string().contains("private text"));
}

#[test]
fn active_epistemic_state_filters_only_named_prompt_records() {
    let memory = [
        json!({"id": "secret_event", "body": "hidden fact"}),
        json!({"id": "current_scene", "body": "derived hidden summary"}),
        json!({"id": "character", "body": "stable character evidence"}),
    ];
    let state = json!({"audit": {"prompt_excluded_memory_ids": [
        "secret_event", "current_scene", "not_retrieved"
    ]}});

    let (filtered, audit) = filter_memory_for_prompt(&memory, &state, true);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0]["id"], "character");
    assert_eq!(audit["applied"], true);
    assert_eq!(audit["excluded_count"], 2);
    assert_eq!(
        audit["excluded_ids"],
        json!(["current_scene", "secret_event"])
    );

    let (shadow, shadow_audit) = filter_memory_for_prompt(&memory, &state, false);
    assert_eq!(shadow.len(), memory.len());
    assert_eq!(shadow_audit["applied"], false);
}

#[test]
fn mo_state_input_audit_binds_projection_without_exposing_bodies() {
    let memory = [json!({"id": "m1", "body": "secret body",
        "memory_space": {"id": "s1"}})];
    let audit = state_input_audit(&memory, &[], "ok").expect("audit");
    assert_eq!(audit["scope"], "retrieved_subset");
    assert_eq!(audit["memory_count"], 1);
    assert_eq!(audit["source_space_ids"], json!(["s1"]));
    assert_eq!(audit["fingerprint_sha256"].as_str().unwrap().len(), 64);
    assert!(!audit.to_string().contains("secret body"));

    let degraded = json!({"context": "unsafe", "audit": {
        "degraded": false, "input": {"retrieval_status": "degraded"}}});
    assert_eq!(
        state_context_for_prompt(&degraded, true, MoStateInjectionMode::Active),
        (String::new(), "suppressed_degraded_input")
    );
}

#[tokio::test]
async fn slow_maintenance_does_not_occupy_foreground_generation_slot() {
    let service = MomoApiService::new(
        "http://127.0.0.1:9/v1",
        None,
        reqwest::Client::new(),
        Arc::new(MomoConfig::default()),
    );
    let maintenance = service.generation_gate("space", "maintenance").await;
    let first = maintenance
        .clone()
        .acquire_owned()
        .await
        .expect("maintenance slot");
    assert!(
        service
            .generation_gate("space", "maintenance")
            .await
            .try_acquire_owned()
            .is_err()
    );
    let foreground = service.generation_gate("space", "foreground").await;
    let reply = foreground
        .clone()
        .try_acquire_owned()
        .expect("foreground must remain available");
    assert!(foreground.clone().try_acquire_owned().is_err());
    drop(first);
    assert!(maintenance.try_acquire_owned().is_ok());
    drop(reply);
    assert!(foreground.try_acquire_owned().is_ok());
}

#[test]
fn roleplay_context_keeps_multi_space_memory_provenance() {
    let rendered = joined_bodies(&[
        json!({
            "id": "preference-rain",
            "body": "The user dislikes rain.",
            "memory_space": {"id": "space-personal", "label": "personal"}
        }),
        json!({
            "id": "scene-rain",
            "body": "It is raining in the shared scene.",
            "memory_space": {"id": "space-group", "label": "group"}
        }),
    ]);

    assert!(rendered.contains("[Memory record: \"preference-rain\" | source: \"personal\"]"));
    assert!(rendered.contains("[Memory record: \"scene-rain\" | source: \"group\"]"));
    assert!(!rendered.contains("space-personal"));
    assert!(!rendered.contains("space-group"));
}

#[test]
fn explicit_drain_uses_configured_maintenance_batch_limits() {
    let mut config = MomoConfig::default();
    config.runtime.memory_distill_every_turns = 7;
    config.runtime.nsg_govern_every_turns = 11;
    let service = MomoApiService::new(
        "http://127.0.0.1:9/v1",
        None,
        reqwest::Client::new(),
        Arc::new(config),
    );
    assert_eq!(service.maintenance_batch_limit(MaintenanceKind::Memory), 7);
    assert_eq!(
        service.maintenance_batch_limit(MaintenanceKind::SemanticGraph),
        11
    );
}
use futures_util::{FutureExt, future::BoxFuture};

#[derive(Debug)]
struct FixedVisionAdapter;

impl VisionDescriptionAdapter for FixedVisionAdapter {
    fn describe(
        &self,
        request: VisionDescriptionRequest,
    ) -> BoxFuture<'static, Result<crate::VisionDescriptionBatch, crate::VisionError>> {
        async move {
            assert_eq!(request.prompt, "Describe visible facts.");
            assert_eq!(request.images.len(), 1);
            Ok(crate::VisionDescriptionBatch {
                descriptions: vec!["A red umbrella on a wet street.".to_owned()],
                usage: ChatUsage {
                    input_tokens: 12,
                    output_tokens: 8,
                    total_tokens: 20,
                },
                upstream_request_ids: vec!["vision-upstream-1".to_owned()],
            })
        }
        .boxed()
    }
}

#[derive(Debug)]
struct UnexpectedVisionAdapter;

impl VisionDescriptionAdapter for UnexpectedVisionAdapter {
    fn describe(
        &self,
        _request: VisionDescriptionRequest,
    ) -> BoxFuture<'static, Result<crate::VisionDescriptionBatch, crate::VisionError>> {
        async move { panic!("direct multimodal input must not call the vision adapter") }.boxed()
    }
}

#[test]
fn cancellation_state_is_active_only_and_drop_safe() {
    let service = MomoApiService::new(
        "http://127.0.0.1:9/v1",
        None,
        reqwest::Client::new(),
        Arc::new(MomoConfig::default()),
    );
    let scope_id = "01900000-0000-7000-8000-000000000101";
    let operation_key = scoped_operation_key(scope_id, "request-1");
    assert!(!service.cancel(scope_id, "request-1"));
    let operation = service.enter_operation(&operation_key, scope_id);
    assert!(service.has_active_responses(scope_id));
    assert!(!service.has_active_responses("01900000-0000-7000-8000-000000000102"));
    assert!(!service.cancel("01900000-0000-7000-8000-000000000102", "request-1"));
    service
        .ensure_active(&operation_key)
        .expect("another scope cannot cancel this operation");
    assert!(service.cancel(scope_id, "request-1"));
    assert!(matches!(
        service.ensure_active(&operation_key),
        Err(MomoApiError {
            kind: MomoApiErrorKind::Cancelled,
            ..
        })
    ));
    drop(operation);
    assert!(!service.has_active_responses(scope_id));
    assert!(!service.cancel(scope_id, "request-1"));
    service
        .ensure_active(&operation_key)
        .expect("dropped operations leave no stale cancellation marker");
}

#[tokio::test]
async fn governed_vision_adapter_resolves_images_to_retryable_text() {
    let mut config = MomoConfig::default();
    config.vision.enabled = true;
    config.vision.prompt = "Describe visible facts.".to_owned();
    let service = MomoApiService::new(
        "http://127.0.0.1:9/v1",
        None,
        reqwest::Client::new(),
        Arc::new(config),
    )
    .with_vision_adapter(Arc::new(FixedVisionAdapter));
    let request: MomoResponseRequest = serde_json::from_value(json!({
        "input": [
            {"type": "input_text", "text": "What do you see?"},
            {"type": "input_image", "image_url": "https://example.test/image.png", "detail": "high"}
        ]
    }))
    .expect("request");
    let governed = service
        .config
        .govern(
            8_192,
            1_024,
            RequestedOverrides {
                context_window: None,
                max_output_tokens: None,
                temperature: None,
                instructions: None,
                visual_description_prompt: None,
                parameters: &serde_json::Map::new(),
                tool_configuration_requested: false,
            },
        )
        .expect("governance");
    let operation = service.enter_operation("request-vision-1", "vision-space");
    let resolved = service
        .resolve_response_input(&request, "request-vision-1", None, &governed, false)
        .await
        .expect("resolved input");
    drop(operation);
    assert_eq!(
        resolved.image_handling,
        ImageInputHandling::DescriptionFallback
    );
    assert_eq!(resolved.visual_input_count, 1);
    assert_eq!(resolved.vision_usage.total_tokens, 20);
    assert_eq!(
        resolved.text,
        "What do you see?\n[Visual description for image 1]\nA red umbrella on a wet street."
    );

    let persisted = json!({
        "resolved_input_json": serde_json::to_string(&resolved).expect("stored resolution")
    });
    let operation = service.enter_operation("request-vision-1", "vision-space");
    let replayed = service
        .resolve_response_input(
            &request,
            "request-vision-1",
            Some(&persisted),
            &governed,
            false,
        )
        .await
        .expect("replayed resolution");
    drop(operation);
    assert_eq!(replayed, resolved);
}

#[tokio::test]
async fn multimodal_chat_uses_original_images_without_vision_prompt() {
    let mut config = MomoConfig::default();
    config.vision.enabled = true;
    config.vision.prompt = "This fallback prompt must not be used.".to_owned();
    let service = MomoApiService::new(
        "http://127.0.0.1:9/v1",
        None,
        reqwest::Client::new(),
        Arc::new(config),
    )
    .with_vision_adapter(Arc::new(UnexpectedVisionAdapter));
    let request: MomoResponseRequest = serde_json::from_value(json!({
        "input": [{
            "type": "message",
            "role": "user",
            "content": [
                {"type": "input_text", "text": "What do you think?"},
                {"type": "input_image", "image_url": "https://example.test/image.png", "detail": "high"}
            ]
        }]
    }))
    .expect("request");
    let governed = service
        .config
        .govern(
            8_192,
            1_024,
            RequestedOverrides {
                context_window: None,
                max_output_tokens: None,
                temperature: None,
                instructions: None,
                visual_description_prompt: Some("Also unused."),
                parameters: &serde_json::Map::new(),
                tool_configuration_requested: false,
            },
        )
        .expect("governance");
    let operation = service.enter_operation("request-direct-vision-1", "direct-vision-space");
    let resolved = service
        .resolve_response_input(&request, "request-direct-vision-1", None, &governed, true)
        .await
        .expect("direct multimodal input");
    drop(operation);
    assert_eq!(
        resolved.image_handling,
        ImageInputHandling::DirectMultimodal
    );
    assert_eq!(resolved.text, "What do you think?\n[Image input: 1]");
    assert_eq!(resolved.vision_usage, ChatUsage::default());
    let messages = request.input.gateway_messages().expect("gateway messages");
    assert!(matches!(
        messages[0].content,
        Some(crate::GatewayMessageContent::Parts(ref parts))
            if parts.iter().any(|part| matches!(part, crate::GatewayContentPart::ImageUrl { .. }))
    ));
}
