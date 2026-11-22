use gpui_agent_contracts::ProjectVerification;
use gpui_agent_presentation::{VerificationDraft,
                              format_command_line,
                              split_command_line};

fn args(values: &[&str]) -> Vec<String> { values.iter().map(|value| (*value).to_owned()).collect() }

#[test]
fn given_command_lines_when_split_then_quotes_group_and_backslashes_stay_literal() {
    assert_eq!(split_command_line("cargo test").unwrap(), args(&["cargo", "test"]));
    assert_eq!(
        split_command_line("  cargo   clippy -- -D warnings ").unwrap(),
        args(&["cargo", "clippy", "--", "-D", "warnings"])
    );
    assert_eq!(
        split_command_line(r#""C:\Program Files\Git\bin\bash.exe" -c 'make check'"#).unwrap(),
        args(&[r"C:\Program Files\Git\bin\bash.exe", "-c", "make check"])
    );
    assert_eq!(split_command_line(r#"--name="a b" """#).unwrap(), args(&["--name=a b", ""]));
    assert!(split_command_line("echo \"open").is_err());
}

#[test]
fn given_any_arguments_when_formatted_and_split_again_then_they_round_trip() {
    for command in [
        args(&["cargo", "test"]),
        args(&[r"C:\Program Files\x.exe", "--flag"]),
        args(&["", "it's", r#"say "hi""#, r#"both ' and ""#]),
    ] {
        assert_eq!(split_command_line(&format_command_line(&command)).unwrap(), command, "{command:?}");
    }
}

#[test]
fn given_a_draft_when_converted_then_blank_lines_are_skipped_and_limits_are_enforced() {
    let saved = ProjectVerification {
        commands: vec![args(&["cargo", "test"])],
        ..Default::default()
    };
    let mut draft = VerificationDraft::from_plan(&saved);
    assert_eq!(draft.commands, ["cargo test"]);
    assert!(!draft.changes(&saved));
    draft.commands.push("   ".into());
    assert!(!draft.changes(&saved), "a blank row is not a change");
    assert_eq!(draft.command_count(), 1);
    draft.commands = vec![];
    assert_eq!(
        draft.to_plan().unwrap().commands,
        Vec::<Vec<String>>::new(),
        "no commands disables verification"
    );
    draft.max_attempts = "11".into();
    assert!(draft.to_plan().is_err());
    draft.max_attempts = "three".into();
    assert!(draft.to_plan().unwrap_err().contains("1~10"));
    draft.max_attempts = "3".into();
    draft.timeout_seconds = "0".into();
    assert!(draft.to_plan().is_err());
    draft.timeout_seconds = "300".into();
    draft.commands = vec!["echo 'open".into()];
    assert!(draft.to_plan().is_err());
    assert!(draft.changes(&saved), "an invalid draft must stay saveable so its error is shown");
}
