use super::*;
use std::fs;

fn paths(root: &Path) -> ExtensionPaths {
    ExtensionPaths {
        jcode_home: root.join(".jcode"),
        user_home: root.join("home"),
        project_dir: Some(root.join("project")),
    }
}

fn skill(dir: &Path, frontmatter: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\n{frontmatter}\n---\n\n# body\n"),
    )
    .unwrap();
}

#[test]
fn scans_all_sources() {
    let root = tempfile::tempdir().unwrap();
    let p = paths(root.path());
    skill(&p.jcode_home.join("skills/alpha"), "name: alpha\ndescription: A skill");
    skill(&p.jcode_home.join("skills/plugin-tools-beta"), "name: beta");
    fs::create_dir_all(p.jcode_home.join("plugins")).unwrap();
    fs::write(
        p.jcode_home.join("plugins/.state.json"),
        r#"{"enabled":{"tools":true},"managed":{"skills":["plugin-tools-beta"],"mcp":[]}}"#,
    )
    .unwrap();
    skill(&p.user_home.join(".agents/skills/shared-one"), "description: shared");
    let project = p.project_dir.as_ref().unwrap();
    skill(&project.join(".jcode/skills/proj"), "name: proj");
    skill(&project.join(".agents/skills/proj-shared"), "name: ps");
    skill(&project.join(".claude/skills/claude-proj"), "name: cp");
    // Disabled skill.
    let off = p.jcode_home.join("skills/turned-off");
    fs::create_dir_all(&off).unwrap();
    fs::write(
        off.join("SKILL.md.disabled"),
        "---\nname: turned-off\n---\n",
    )
    .unwrap();
    // A dir without any SKILL.md* is ignored.
    fs::create_dir_all(p.jcode_home.join("skills/not-a-skill")).unwrap();
    // Claude plugin install.
    let install = p.user_home.join(".claude/plugins/cache/mp/plug/1.0");
    skill(&install.join("skills/plug-skill"), "name: plug-skill");
    fs::create_dir_all(p.user_home.join(".claude/plugins")).unwrap();
    fs::write(
        p.user_home.join(".claude/plugins/installed_plugins.json"),
        &format!(
            r#"{{"plugins":{{"plug":{{"installPath":"{}"}}}}}}"#,
            install.to_string_lossy().replace('\\', "\\\\")
        ),
    )
    .unwrap();

    let skills = list_skills(&p);
    let by_name: std::collections::HashMap<_, _> = skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill))
        .collect();
    assert_eq!(by_name.len(), 8, "{:?}", by_name.keys().collect::<Vec<_>>());
    assert_eq!(by_name["alpha"].source, SkillSource::Jcode);
    assert_eq!(by_name["alpha"].description, "A skill");
    assert!(by_name["alpha"].editable && by_name["alpha"].enabled);
    assert_eq!(
        by_name["beta"].source,
        SkillSource::Plugin("tools".into())
    );
    assert!(!by_name["beta"].editable);
    assert!(by_name["shared-one"].shared);
    assert_eq!(by_name["proj"].source, SkillSource::ProjectJcode);
    assert_eq!(by_name["ps"].source, SkillSource::ProjectAgents);
    assert!(by_name["ps"].shared);
    assert_eq!(by_name["cp"].source, SkillSource::ProjectClaude);
    assert!(!by_name["cp"].editable);
    assert!(!by_name["turned-off"].enabled);
    assert_eq!(by_name["plug-skill"].source, SkillSource::ClaudePlugin);
    assert!(!by_name["plug-skill"].editable);
}

#[test]
fn frontmatter_fallback_and_quotes() {
    let root = tempfile::tempdir().unwrap();
    let p = paths(root.path());
    skill(
        &p.jcode_home.join("skills/dir-name"),
        "description: \"quoted desc\"",
    );
    let skills = list_skills(&p);
    assert_eq!(skills[0].name, "dir-name");
    assert_eq!(skills[0].description, "quoted desc");
}

#[test]
fn enable_disable_renames() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("skills/x");
    skill(&dir, "name: x");
    let entry = SkillEntry {
        name: "x".into(),
        description: String::new(),
        dir: dir.clone(),
        source: SkillSource::Jcode,
        enabled: true,
        editable: true,
        shared: false,
    };
    set_skill_enabled(&entry, false).unwrap();
    assert!(dir.join("SKILL.md.disabled").is_file());
    assert!(!dir.join("SKILL.md").exists());
    let entry = SkillEntry { enabled: false, ..entry };
    set_skill_enabled(&entry, true).unwrap();
    assert!(dir.join("SKILL.md").is_file());
}

#[test]
fn create_validation_and_stub() {
    let root = tempfile::tempdir().unwrap();
    let root_dir = root.path().join("skills");
    assert!(!valid_skill_name(""));
    assert!(!valid_skill_name("-lead"));
    assert!(!valid_skill_name("UPPER"));
    assert!(!valid_skill_name("has space"));
    assert!(!valid_skill_name(&"a".repeat(65)));
    assert!(valid_skill_name("ok-name-1"));
    assert!(create_skill(&root_dir, "Bad Name", "").is_err());
    let dir = create_skill(&root_dir, "new-skill", "Does a thing").unwrap();
    let text = fs::read_to_string(dir.join("SKILL.md")).unwrap();
    assert!(text.contains("name: new-skill"));
    assert!(text.contains("description: Does a thing"));
    assert!(text.contains("# new-skill"));
    assert!(create_skill(&root_dir, "new-skill", "").is_err());
}

#[test]
fn import_copies_and_refuses() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("elsewhere/mover");
    skill(&src, "name: mover");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("sub/nested.txt"), "x").unwrap();
    let dest_root = root.path().join("skills");
    let dest = import_skill(&src, &dest_root).unwrap();
    assert!(dest.join("SKILL.md").is_file());
    assert_eq!(fs::read_to_string(dest.join("sub/nested.txt")).unwrap(), "x");
    assert!(import_skill(&src, &dest_root).is_err(), "exists");
    assert!(
        import_skill(&root.path().join("elsewhere"), &dest_root).is_err(),
        "no SKILL.md"
    );
}

#[test]
fn delete_moves_to_trash() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("skills/gone");
    skill(&dir, "name: gone");
    let entry = SkillEntry {
        name: "gone".into(),
        description: String::new(),
        dir: dir.clone(),
        source: SkillSource::Jcode,
        enabled: true,
        editable: true,
        shared: false,
    };
    let target = delete_skill(&entry, root.path()).unwrap();
    assert!(!dir.exists());
    assert!(target.join("SKILL.md").is_file());
    assert!(target.starts_with(root.path().join("trash/skills")));
}
