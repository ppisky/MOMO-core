use super::*;

#[test]
fn parses_structured_scene_without_inference() {
    let scene = "# 当前场景\n\n## Scene ID\nscene_home\n\n## Status\nactive\n\n## Location\n客厅\n\n## Participants\n- momo\n- user\n\n## Focus\n讨论出行\n\n## Constraints\n- 保持安静 [[rule_quiet]]\n";
    let snapshot = parse_scene(scene, "# 活跃剧情线\n\n- 等待确认目的地\n", "hash");
    assert_eq!(snapshot.scene_id, "scene_home");
    assert_eq!(snapshot.status, SceneStatus::Active);
    assert_eq!(snapshot.location.as_deref(), Some("客厅"));
    assert_eq!(snapshot.participants, vec!["momo", "user"]);
    assert_eq!(snapshot.open_threads, vec!["等待确认目的地"]);
    assert_eq!(snapshot.source_refs, vec!["rule_quiet"]);
}

#[test]
fn english_control_keys_preserve_multilingual_scene_values() {
    for (location, focus, constraint) in [
        ("客厅", "讨论出行", "保持安静"),
        ("居間", "旅程について話す", "静かにする"),
        ("غرفة المعيشة", "مناقشة الرحلة", "التزام الهدوء"),
    ] {
        let scene = format!(
            "## Scene ID\nscene_home\n\n## Status\nactive\n\n## Location\n{location}\n\n## Focus\n{focus}\n\n## Constraints\n- {constraint}\n"
        );
        let snapshot = parse_scene(&scene, "", "hash");
        assert_eq!(snapshot.location.as_deref(), Some(location));
        assert_eq!(snapshot.focus.as_deref(), Some(focus));
        assert_eq!(snapshot.constraints, [constraint]);
    }
}

#[test]
fn localized_control_keys_are_not_protocol_aliases() {
    let scene = "## 状态\n未开始\n\n## 地点\n客厅\n";
    let snapshot = parse_scene(scene, "", "hash");
    assert_eq!(snapshot.status, SceneStatus::Inactive);
    assert!(snapshot.location.is_none());
}
