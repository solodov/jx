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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands) enum PullRequestTableLayout {
    Flow,
    FitTerminal,
}

pub(in crate::commands) fn render_elastic_table_row(
    prefix: &str,
    title: &str,
    suffix: &str,
    right: &str,
    terminal_width: Option<usize>,
) -> String {
    let Some(terminal_width) = terminal_width else {
        return flow_table_row(prefix, title, suffix, right);
    };

    let prefix_width = rendered_visible_width(prefix);
    let suffix_width = rendered_visible_width(suffix);
    let title_suffix_gap = usize::from(!suffix.is_empty());
    let right_gap = usize::from(!right.is_empty()) * 2;
    let right_width = rendered_visible_width(right);
    let right_margin = usize::from(!right.is_empty());
    let title_width = terminal_width.saturating_sub(
        prefix_width + title_suffix_gap + suffix_width + right_gap + right_width + right_margin,
    );
    let title = ellipsize_rendered_line(title, Some(title_width));

    let title_suffix_gap = if suffix.is_empty() { "" } else { " " };
    let left = format!("{prefix}{title}{title_suffix_gap}{suffix}");
    if right.is_empty() {
        return ellipsize_rendered_line(&left, Some(terminal_width));
    }

    let used_width = rendered_visible_width(&left) + right_width + right_margin;
    let gap = terminal_width.saturating_sub(used_width);
    let line = format!(
        "{left}{}{right}{}",
        " ".repeat(gap),
        " ".repeat(right_margin)
    );
    ellipsize_rendered_line(&line, Some(terminal_width))
}

pub(in crate::commands) fn flow_table_row(
    prefix: &str,
    title: &str,
    suffix: &str,
    right: &str,
) -> String {
    let mut parts = vec![title.to_owned()];
    if !suffix.is_empty() {
        parts.push(suffix.to_owned());
    }
    if !right.is_empty() {
        parts.push(right.to_owned());
    }
    format!("{prefix}{}", parts.join(" "))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elastic_table_row_shrinks_title_before_right_metadata() {
        let row = render_elastic_table_row(
            "  #12      ✓    ?    <1h   ",
            "Implement a very long synthetic pull request title",
            "[workflow]",
            "Example Reviewer",
            Some(72),
        );

        assert_eq!(rendered_visible_width(&row), 72);
        assert!(row.ends_with("Example Reviewer "));
        assert!(row.contains("… [workflow]"));
        assert!(!row.contains("request title"));
    }

    #[test]
    fn elastic_table_row_right_aligns_metadata_when_title_fits() {
        let row = render_elastic_table_row(
            "  #12      ✓    ?    <1h   ",
            "Short title",
            "[workflow]",
            "Example Reviewer",
            Some(72),
        );

        assert_eq!(rendered_visible_width(&row), 72);
        assert!(row.ends_with("Example Reviewer "));
        assert!(row.contains("Short title [workflow]"));
    }
}
