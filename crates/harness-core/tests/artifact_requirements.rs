use harness_core::{
    PermissionPolicy, ToolRegistry,
    artifact_requirements::{ArtifactRequirements, CheckOutcome},
    task_contract::{TaskContract, TaskOutcome, TaskResult},
    verification::{FileEvidenceVerifier, SuccessCriterion, Verifier},
};
use serde_json::json;

fn requirements() -> ArtifactRequirements {
    ArtifactRequirements {
        path: "report.md".into(),
        min_content_chars: 10,
        required_headings: vec!["Summary".into(), "Sources".into()],
        source_references: vec!["https://example.com/research".into()],
    }
}
fn report() -> &'static str {
    "# Summary\nA useful summary of the findings.\n## Sources\n[Source][research]\n\n[research]: https://example.com/research\n"
}
fn contract() -> TaskContract {
    TaskContract {
        goal: "Research and save a report".into(),
        criteria: vec![SuccessCriterion::ArtifactRequirements(requirements())],
    }
}

#[test]
fn report_checks_real_bytes_and_never_certifies_source_quality() {
    let root = tempfile::tempdir().unwrap();
    let policy = PermissionPolicy::milestone_default(root.path());
    let contract = contract();
    contract.validate(&contract.goal).unwrap();
    contract.validate_scope(&policy).unwrap();
    let missing = contract.verify(&policy);
    assert!(matches!(missing.outcome, TaskOutcome::Unmet));
    assert_eq!(missing.checks[0].outcome, Some(CheckOutcome::Inconclusive));
    std::fs::write(root.path().join("report.md"), report()).unwrap();
    let passed = contract.verify(&policy);
    assert!(matches!(passed.outcome, TaskOutcome::Verified));
    assert!(
        passed.checks[0]
            .evidence
            .contains("source quality are not verified")
    );
    assert_eq!(passed.checks[0].outcome, Some(CheckOutcome::Passed));
    std::fs::write(
        root.path().join("report.md"),
        report().replace("# Summary", "# Different"),
    )
    .unwrap();
    assert_eq!(
        contract.verify(&policy).checks[0].outcome,
        Some(CheckOutcome::Failed)
    );
    std::fs::write(root.path().join("report.md"), [0xff, 0xfe]).unwrap();
    assert_eq!(
        contract.verify(&policy).checks[0].outcome,
        Some(CheckOutcome::Inconclusive)
    );
    std::fs::write(
        root.path().join("report.md"),
        "x".repeat(harness_core::tools::MAX_FILE_BYTES + 1),
    )
    .unwrap();
    assert_eq!(
        contract.verify(&policy).checks[0].outcome,
        Some(CheckOutcome::Inconclusive)
    );
}

#[test]
fn markdown_syntax_not_substring_matching_defines_headings_and_links() {
    let requirements = requirements();
    let setext = "Summary\n=======\nA useful summary of findings.\n## **Sources**\n<https://example.com/research>";
    assert_eq!(requirements.check(setext).0, CheckOutcome::Passed);
    for fake in [
        format!("```markdown\n{}\n```", report()),
        format!("<!--\n{}\n-->", report()),
        format!("<pre>\n{}\n</pre>", report()),
        report()
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect(),
        report().replace("[Source][research]", "![Source][research]"),
        report().replace("[Source][research]", "`https://example.com/research`"),
        report().replace("[Source][research]", "https://example.com/research"),
        report().replace("https://example.com/research", "https://example.com/other"),
        report().replace("# Summary", "Summary is mentioned in prose."),
    ] {
        assert_ne!(requirements.check(&fake).0, CheckOutcome::Passed, "{fake}");
    }
    let only_headers = "# Summary\n## Sources\n[x](https://example.com/research)";
    assert_eq!(requirements.check(only_headers).0, CheckOutcome::Failed);
    assert_eq!(
        requirements
            .check(
                &report()
                    .lines()
                    .map(|line| format!("> {line}\n"))
                    .collect::<String>()
            )
            .0,
        CheckOutcome::Failed
    );
    assert_eq!(
        requirements
            .check(&format!("<span hidden>{}</span>", report()))
            .0,
        CheckOutcome::Inconclusive
    );
}

#[test]
fn criteria_are_bounded_and_cannot_escape_scope_or_inject_evaluators() {
    for path in [
        "../report.md",
        "/report.md",
        "C:/report.md",
        "a/../report.md",
        "./report.md",
        "a//report.md",
        "a\\report.md",
        "report.txt",
        "a/ report.md",
    ] {
        let mut item = requirements();
        item.path = path.into();
        assert!(item.validate().is_err(), "{path}");
    }
    for source in [
        "javascript:alert(1)",
        "http://example.com/",
        "https://user:pass@example.com/",
        "https://example.com:99999/",
        "https://example.com/a b",
        "https://",
    ] {
        let mut item = requirements();
        item.source_references = vec![source.into()];
        assert!(item.validate().is_err(), "{source}");
    }
    let mut item = requirements();
    item.required_headings.push("Summary".into());
    assert!(item.validate().is_err());
    item = requirements();
    item.min_content_chars = 0;
    assert!(item.validate().is_err());
    item = requirements();
    item.source_references = vec!["https://example.com/".into(); 33];
    assert!(item.validate().is_err());
    item = requirements();
    item.source_references = vec![format!("https://example.com/{}", "x".repeat(2048))];
    assert!(item.validate().is_err());
    let mut raw = serde_json::to_value(requirements()).unwrap();
    raw["evaluator"] = json!("run arbitrary code");
    assert!(serde_json::from_value::<ArtifactRequirements>(raw).is_err());
    let root = tempfile::tempdir().unwrap();
    let mut policy = PermissionPolicy::milestone_default(root.path());
    policy.revoke_capability(harness_core::Capability::FilesystemRead);
    assert!(requirements().validate_scope(&policy).is_err());
    assert!(matches!(
        contract().verify(&policy).outcome,
        TaskOutcome::Unmet
    ));
}

#[test]
fn unicode_counts_characters_and_legacy_results_keep_their_meaning() {
    let mut item = requirements();
    item.min_content_chars = 2;
    item.source_references.clear();
    assert_eq!(
        item.check("# Summary\n## Sources\n語").0,
        CheckOutcome::Failed
    );
    assert_eq!(
        item.check("# Summary\n## Sources\n語 文").0,
        CheckOutcome::Passed
    );
    let legacy = json!({"outcome":"reviewed","checks":[]});
    let decoded: TaskResult = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    let legacy = json!({"outcome":"verified","checks":[{"criterion":0,"passed":true,"evidence":"old evidence"}]});
    let decoded: TaskResult = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("old.txt"), "unchanged").unwrap();
    let old = TaskContract::from_goal("write file old.txt :: unchanged").unwrap();
    assert!(matches!(
        old.verify(&PermissionPolicy::milestone_default(root.path()))
            .outcome,
        TaskOutcome::Verified
    ));
}

#[test]
fn generic_verifier_rejects_unbound_and_failed_observations() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("report.md"), report()).unwrap();
    let criterion = SuccessCriterion::ArtifactRequirements(requirements());
    let action = criterion.observation_action();
    let mut observed = ToolRegistry::milestone_default()
        .execute(&action, &PermissionPolicy::milestone_default(root.path()));
    assert!(
        FileEvidenceVerifier
            .verify(&criterion, &action, &observed)
            .passed
    );
    assert!(
        !FileEvidenceVerifier
            .verify(
                &criterion,
                &harness_core::Action::ReadFile {
                    path: "other.md".into()
                },
                &observed
            )
            .passed
    );
    observed.ok = false;
    assert!(
        !FileEvidenceVerifier
            .verify(&criterion, &action, &observed)
            .passed
    );
}
