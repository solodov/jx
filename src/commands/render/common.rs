use super::*;

pub(in crate::commands) fn render_linked_output(
    current_dir: &Path,
    color: bool,
    render: impl FnOnce(&mut dyn Formatter) -> io::Result<()>,
) -> Result<String, JjError> {
    if color {
        JjWorkspace::render_workspace_formatted_output(current_dir, render)
    } else {
        Ok(render_plain_output(render))
    }
}

pub(in crate::commands) fn render_plain_output(
    render: impl FnOnce(&mut dyn Formatter) -> io::Result<()>,
) -> String {
    let mut output = Vec::new();
    let mut formatter = PlainTextFormatter::new(&mut output);
    render(&mut formatter).expect("writing command output to a string cannot fail");
    String::from_utf8(output).expect("command output is UTF-8")
}
