use multi_agent_on_chat::{domain::*, engine::CodexEngine, skills, store::Store};
use serde_json::json;
#[test]
fn names_resolve_uniquely_and_unlisted_skills_are_disabled() {
    let list = vec![
        json!({"name":"same","path":"/a/SKILL.md","enabled":true}),
        json!({"name":"same","path":"/b/SKILL.md","enabled":true}),
    ];
    assert!(skills::overrides(&list, &["same".into()]).is_err());
    let result = skills::overrides(&list, &["/a/SKILL.md".into()]).unwrap();
    assert_eq!(result[0]["enabled"], true);
    assert_eq!(result[1]["enabled"], false);
    assert!(skills::verify(&list, &["/a/SKILL.md".into()]).is_err());
    assert!(skills::toml_overrides(&result)
        .unwrap()
        .contains("enabled=false"));
    let t: Template = serde_json::from_value(json!({"id":"old","skills":[]})).unwrap();
    assert_eq!(t.skills_mode, SkillsMode::Inherit);
}
#[tokio::test]
async fn management_is_per_agent_and_template_snapshots_preserve_policy() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path()).unwrap();
    for id in ["a", "b"] {
        s.add_bot_with_defaults(&Bot {
            id: id.into(),
            channel: "local".into(),
            account_id: id.into(),
            credential_ref: None,
            enabled: true,
            owner: Some("local".into()),
        })
        .unwrap();
    }
    let a: Agent = s.get("agent", "a-main").unwrap();
    let path = a.workspace.join(".agents/skills/mac-fixture-skill");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("SKILL.md"),
        "---\nname: mac-fixture-skill\ndescription: Fixture only\n---\nTest",
    )
    .unwrap();
    skills::command(&s, "a-main", "mode allowlist")
        .await
        .unwrap();
    let empty = skills::command(&s, "a-main", "list").await.unwrap();
    assert!(empty["skills"].as_array().unwrap().is_empty());
    skills::command(&s, "a-main", "enable mac-fixture-skill")
        .await
        .unwrap();
    let list = skills::command(&s, "a-main", "list").await.unwrap();
    assert_eq!(list["skills"].as_array().unwrap().len(), 1);
    assert_eq!(list["skills"][0]["name"], "mac-fixture-skill");
    let updated: Agent = s.get("agent", "a-main").unwrap();
    let rev = updated.revision;
    assert!(
        skills::command(&s, "a-main", "enable no-such-fixture-skill")
            .await
            .is_err()
    );
    assert_eq!(s.get::<Agent>("agent", "a-main").unwrap().revision, rev);
    assert_eq!(
        s.get::<Agent>("agent", "b-main")
            .unwrap()
            .config
            .skills_mode,
        SkillsMode::Inherit
    );
    let template = s
        .template_from_agent("a-main", "selected", 1, None)
        .unwrap();
    assert_eq!(template.skills_mode, SkillsMode::Allowlist);
    assert_eq!(template.skills, updated.config.skills);
    skills::command(&s, "a-main", "disable mac-fixture-skill")
        .await
        .unwrap();
    assert_eq!(
        s.get::<Template>("template", "selected@1")
            .unwrap()
            .skills
            .len(),
        1
    );
    skills::command(&s, "a-main", "mode inherit").await.unwrap();
}
#[tokio::test]
async fn real_engine_enforces_empty_allowlist_and_does_not_change_sibling_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let mut template: Template =
        serde_json::from_value(json!({"id":"test","skills_mode":"allowlist"})).unwrap();
    let skill = dir.path().join(".agents/skills/runtime-fixture");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: runtime-fixture\ndescription: fixture\n---\nTest",
    )
    .unwrap();
    let engine = CodexEngine::connect(&template, dir.path(), None)
        .await
        .unwrap();
    let visible = engine
        .control()
        .request(
            "skills/list",
            json!({"cwds":[dir.path()],"forceReload":true}),
        )
        .await
        .unwrap();
    assert!(skills::catalog(visible)
        .unwrap()
        .iter()
        .all(|s| s["enabled"] == false));
    engine.shutdown().await.unwrap();
    template.skills = vec!["runtime-fixture".into()];
    let engine = CodexEngine::connect(&template, dir.path(), None)
        .await
        .unwrap();
    let selected = skills::catalog(
        engine
            .control()
            .request("skills/list", json!({"cwds":[dir.path()]}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(selected.iter().filter(|s| s["enabled"] == true).count(), 1);
    engine.shutdown().await.unwrap();
    let all = CodexEngine::skills_catalog("codex", &template, dir.path(), &[])
        .await
        .unwrap();
    assert!(all
        .iter()
        .any(|s| s["name"] == "runtime-fixture" && s["enabled"] == true));
}
