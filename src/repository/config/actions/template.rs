/// Expands known placeholders once, preserving argument boundaries and supporting literal `{{`/`}}`.
pub(crate) fn render_pr_action_argument(
    template: &str,
    mut value: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                output.push('{');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) => name.push(ch),
                        None => return Err("unclosed action placeholder".to_owned()),
                    }
                }
                if !matches!(
                    name.as_str(),
                    "repo"
                        | "repo_root"
                        | "pr_number"
                        | "pr_url"
                        | "title"
                        | "branch"
                        | "base_branch"
                        | "head_oid"
                        | "local_commit_id"
                        | "local_change_id"
                ) {
                    return Err(format!("unknown action placeholder `{{{name}}}`"));
                }
                output.push_str(&value(&name)?);
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                output.push('}');
            }
            '}' => {
                return Err(
                    "unescaped `}` in action argument; use `}}` for a literal brace".to_owned(),
                )
            }
            ch => output.push(ch),
        }
    }
    Ok(output)
}
