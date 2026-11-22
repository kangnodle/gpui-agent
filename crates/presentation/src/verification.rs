//! Editable form of a project's required verification plan.
//!
//! Commands are edited one per line and stored as argument arrays. The host
//! runs them without a shell, so the line syntax only groups arguments: quotes
//! (`"` or `'`) keep spaces together and backslashes stay literal so Windows
//! paths survive unchanged.
use gpui_agent_contracts::ProjectVerification;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationDraft {
    pub commands: Vec<String>,
    pub max_attempts: String,
    pub timeout_seconds: String,
}

impl VerificationDraft {
    pub fn from_plan(plan: &ProjectVerification) -> Self {
        Self {
            commands: plan.commands.iter().map(|command| format_command_line(command)).collect(),
            max_attempts: plan.max_attempts.to_string(),
            timeout_seconds: plan.timeout_seconds.to_string(),
        }
    }

    /// Blank lines are ignored; every other line must parse and the whole plan
    /// must pass the host's limits.
    pub fn to_plan(&self) -> Result<ProjectVerification, String> {
        let commands = self
            .commands
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| split_command_line(line).map_err(str::to_owned))
            .collect::<Result<Vec<_>, _>>()?;
        let max_attempts = self
            .max_attempts
            .trim()
            .parse()
            .map_err(|_| "최대 시도 횟수는 1~10 사이의 숫자로 입력하세요.".to_owned())?;
        let timeout_seconds = self
            .timeout_seconds
            .trim()
            .parse()
            .map_err(|_| "제한 시간은 1~3600초 사이의 숫자로 입력하세요.".to_owned())?;
        let plan = ProjectVerification {
            commands,
            max_attempts,
            timeout_seconds,
        };
        plan.validate()?;
        Ok(plan)
    }

    /// Whether saving would change the stored plan. An unparsable draft
    /// counts as changed so the user can see the save error.
    pub fn changes(&self, saved: &ProjectVerification) -> bool { self.to_plan().as_ref() != Ok(saved) }

    pub fn command_count(&self) -> usize { self.commands.iter().filter(|line| !line.trim().is_empty()).count() }
}

pub fn split_command_line(line: &str) -> Result<Vec<String>, &'static str> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quote = None;
    for c in line.chars() {
        match quote {
            | Some(open) if c == open => quote = None,
            | Some(_) => current.push(c),
            | None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_arg = true;
            },
            | None if c.is_whitespace() =>
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                },
            | None => {
                current.push(c);
                in_arg = true;
            },
        }
    }
    if quote.is_some() {
        return Err("따옴표가 닫히지 않은 명령이 있습니다.");
    }
    if in_arg {
        args.push(current);
    }
    Ok(args)
}

/// Inverse of [`split_command_line`]: arguments that would split or vanish
/// are quoted.
pub fn format_command_line(args: &[String]) -> String { args.iter().map(|arg| quote_arg(arg)).collect::<Vec<_>>().join(" ") }

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'') {
        return arg.to_owned();
    }
    if !arg.contains('"') {
        return format!("\"{arg}\"");
    }
    if !arg.contains('\'') {
        return format!("'{arg}'");
    }
    // Both quote kinds: alternate quoted segments, which concatenate into one
    // argument.
    let mut out = String::new();
    for (index, part) in arg.split('"').enumerate() {
        if index > 0 {
            out.push_str("'\"'");
        }
        if !part.is_empty() {
            out.push('"');
            out.push_str(part);
            out.push('"');
        }
    }
    out
}
