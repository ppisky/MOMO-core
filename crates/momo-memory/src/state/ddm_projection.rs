//! Adapter from one MO State snapshot to DDM's governed signal contract.

use crate::{
    ddm::{DdmAudit, DdmBand, DdmSignalSnapshot, DdmSignalValue},
    nsg::RetrievedNsg,
};

use super::{DdmRuntimeInput, DmwSignal, normalize};

pub(super) fn signals(
    memory: &[DmwSignal],
    nsg: &[RetrievedNsg],
    state: &[(String, Vec<String>)],
    runtime: Option<&DdmRuntimeInput>,
) -> DdmSignalSnapshot {
    let mut output = DdmSignalSnapshot::default();
    for signal in memory {
        output.insert(
            format!("dmw.kind.{}", normalize(&signal.kind)),
            DdmSignalValue::Bool(true),
            format!("dmw:{}", signal.id),
        );
        for tag in &signal.tags {
            output.insert(
                format!("dmw.tag.{tag}"),
                DdmSignalValue::Bool(true),
                format!("dmw:{}", signal.id),
            );
        }
    }
    for node in nsg {
        output.insert(
            format!("nsg.node.{}", normalize(&node.id)),
            DdmSignalValue::Bool(true),
            format!("nsg:{}", node.id),
        );
    }
    for (dimension, _) in state {
        output.insert(
            format!("state.dimension.{dimension}"),
            DdmSignalValue::Bool(true),
            format!("state:{dimension}"),
        );
    }
    if let Some(runtime) = runtime {
        if let Some(scene) = runtime.scene.as_ref() {
            output.insert(
                "scene.status",
                DdmSignalValue::Text(
                    match scene.status {
                        crate::SceneStatus::Inactive => "inactive",
                        crate::SceneStatus::Active => "active",
                        crate::SceneStatus::Transitioning => "transitioning",
                        crate::SceneStatus::Closed => "closed",
                    }
                    .to_owned(),
                ),
                format!("scene:{}", scene.source_hash),
            );
            for participant in &scene.participants {
                if let Some(participant) = signal_component(participant) {
                    output.insert(
                        format!("scene.participant.{participant}"),
                        DdmSignalValue::Bool(true),
                        format!("scene:{}", scene.source_hash),
                    );
                }
            }
            for source_ref in &scene.source_refs {
                if let Some(source_ref) = signal_component(source_ref) {
                    output.insert(
                        format!("scene.source_ref.{source_ref}"),
                        DdmSignalValue::Bool(true),
                        format!("scene:{}", scene.source_hash),
                    );
                }
            }
        }
        if matches!(
            runtime.request_event_type.as_str(),
            "user_message" | "tool_result"
        ) {
            let evidence = if runtime.request_evidence_id.trim().is_empty() {
                "request:current".to_owned()
            } else {
                format!("request:{}", runtime.request_evidence_id)
            };
            output.insert(
                "request.event_type",
                DdmSignalValue::Text(runtime.request_event_type.clone()),
                evidence.clone(),
            );
            output.insert(
                "request.has_image",
                DdmSignalValue::Bool(runtime.request_has_image),
                evidence,
            );
        }
    }
    output
}

fn signal_component(value: &str) -> Option<String> {
    let value = normalize(value)
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-' | '.' | ':')
            {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('_').to_owned();
    (!value.is_empty() && value.len() <= 128).then_some(value)
}

pub(super) fn context_cues(audit: &DdmAudit) -> Vec<String> {
    let mut output = audit
        .constraints
        .iter()
        .map(|constraint| format!("Constraint — {}", constraint.trim()))
        .collect::<Vec<_>>();
    output.extend(audit.effective_dispositions.iter().map(|disposition| {
        let band = match disposition.band {
            DdmBand::Latent => "Latent",
            DdmBand::Salient => "Salient",
            DdmBand::Dominant => "Dominant",
        };
        format!("{band} — {}: {}", disposition.id, disposition.cue)
    }));
    output
}
