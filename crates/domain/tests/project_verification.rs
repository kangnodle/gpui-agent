use gpui_agent_domain::ProjectVerification;
use serde_json::json;

#[test]
fn given_required_checks_when_a_project_is_configured_then_only_bounded_executable_argument_lists_are_accepted() {
    let plan: ProjectVerification = serde_json::from_value(json!({"commands":[["bun","test","server"],["vp","check"]]})).unwrap();
    assert!(plan.validate().is_ok());
    assert_eq!(plan.max_attempts, 3);
    for value in [
        json!({"commands":[[]]}),
        json!({"commands":[[" "]]}),
        json!({"commands":[["bun","test\u{0000}"]]}),
        json!({"commands":[],"max_attempts":0}),
    ] {
        assert!(serde_json::from_value::<ProjectVerification>(value).unwrap().validate().is_err());
    }
}

#[test]
fn given_command_receipts_when_completion_is_decided_then_every_command_must_pass_on_the_same_code_version() {
    use gpui_agent_domain::{CodeVersion,
                            VerificationCheck,
                            VerificationReport,
                            VerificationStatus};
    let version = CodeVersion {
        head: Some("commit-a".into()),
        fingerprint: "working-tree-a".into(),
    };
    let mut report = VerificationReport {
        job_id: 7,
        attempt: 1,
        revision: version.clone(),
        finished_revision: Some(version.clone()),
        checks: vec![VerificationCheck {
            command: vec!["bun".into(), "test".into()],
            status: VerificationStatus::Passed,
            exit_code: Some(0),
            output: "2 passed".into(),
        }],
    };
    assert!(report.passed());
    report.checks[0].exit_code = Some(1);
    assert!(!report.passed());
    report.checks[0].exit_code = Some(0);
    report.finished_revision.as_mut().unwrap().fingerprint = "changed-during-tests".into();
    assert!(!report.passed());
    report.finished_revision = None;
    assert!(!report.passed());
}
