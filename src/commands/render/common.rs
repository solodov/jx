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

pub(in crate::commands) fn ellipsize_rendered_line(line: &str, max_width: Option<usize>) -> String {
    let Some(max_width) = max_width else {
        return line.to_owned();
    };
    if rendered_visible_width(line) <= max_width {
        return line.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }

    let target_width = max_width.saturating_sub(1);
    let mut rendered = String::new();
    let mut width = 0usize;
    let mut rest = line;
    let mut open_osc8 = false;
    let mut copied_escape_sequence = false;
    while !rest.is_empty() && width < target_width {
        if let Some(sequence) = ansi_sequence_prefix(rest) {
            copied_escape_sequence = true;
            if let Some(osc8_open) = osc8_open_state(sequence) {
                open_osc8 = osc8_open;
            }
            rendered.push_str(sequence);
            rest = &rest[sequence.len()..];
            continue;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        rendered.push(ch);
        width += 1;
        rest = &rest[ch.len_utf8()..];
    }
    rendered.push('…');
    if open_osc8 {
        rendered.push_str("\x1b]8;;\x1b\\");
    }
    if copied_escape_sequence {
        rendered.push_str(RESET_STYLE);
    }
    rendered
}

pub(in crate::commands) fn rendered_visible_width(line: &str) -> usize {
    let mut width = 0usize;
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(sequence) = ansi_sequence_prefix(rest) {
            rest = &rest[sequence.len()..];
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        width += 1;
        rest = &rest[ch.len_utf8()..];
    }
    width
}

fn ansi_sequence_prefix(value: &str) -> Option<&str> {
    if value.starts_with("\x1b[") {
        return csi_sequence_prefix(value);
    }
    if value.starts_with("\x1b]") {
        return osc_sequence_prefix(value);
    }
    None
}

fn csi_sequence_prefix(value: &str) -> Option<&str> {
    let end = value
        .bytes()
        .enumerate()
        .skip(2)
        .find_map(|(index, byte)| (0x40..=0x7e).contains(&byte).then_some(index + 1))?;
    Some(&value[..end])
}

fn osc_sequence_prefix(value: &str) -> Option<&str> {
    if let Some(index) = value.find('\x07') {
        return Some(&value[..=index]);
    }
    let index = value.find("\x1b\\")?;
    Some(&value[..index + 2])
}

fn osc8_open_state(sequence: &str) -> Option<bool> {
    let body = sequence.strip_prefix("\x1b]")?;
    let body = body
        .strip_suffix("\x1b\\")
        .or_else(|| body.strip_suffix('\x07'))?;
    let payload = body.strip_prefix("8;;")?;
    Some(!payload.is_empty())
}
