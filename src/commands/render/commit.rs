use super::*;
use jj_cli::formatter::FormatterExt as _;

/// Renders commit descriptions and full changed-file paths with one layout for status and publish.
pub(in crate::commands) fn render_commit_content(
    description: &str,
    change_lines: &[String],
    current_dir: &Path,
    output: OutputMode,
) -> Result<String, JjError> {
    let width = output
        .terminal_width
        .unwrap_or_else(|| termimad::terminal_size().0.into());
    let mut blocks = Vec::new();
    if !description.trim_end().is_empty() {
        blocks.push(
            render_commit_description(description.trim_end(), width)
                .trim_end()
                .to_owned(),
        );
    }
    if !change_lines.is_empty() {
        blocks.push(render_file_changes(
            change_lines,
            current_dir,
            output.color,
        )?);
    }
    Ok(blocks.join("\n\n"))
}

/// Keeps native jj styling intact and gives plain plan/revision facts the same jj diff labels.
fn render_file_changes(
    lines: &[String],
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    if !color || !lines.iter().any(|line| file_change_label(line).is_some()) {
        return Ok(lines.join("\n"));
    }
    render_linked_output(current_dir, true, |formatter| {
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            if let Some(label) = file_change_label(line) {
                write!(
                    formatter.labeled("diff").labeled("summary").labeled(label),
                    "{line}"
                )?;
            } else {
                // Status may include native ANSI, warnings, or untracked-path details.
                formatter.raw()?.write_all(line.as_bytes())?;
            }
        }
        Ok(())
    })
}

fn file_change_label(line: &str) -> Option<&'static str> {
    if line.contains('\x1b') {
        return None;
    }
    match line.split_once(' ')?.0 {
        "A" => Some("added"),
        "M" => Some("modified"),
        "D" => Some("removed"),
        "R" => Some("renamed"),
        "C" => Some("copied"),
        "?" => Some("untracked"),
        _ => None,
    }
}

/// Renders authored Markdown while projecting generated PR stack blocks for terminal output.
fn render_commit_description(description: &str, width: usize) -> String {
    let sections = domain::pull_request_description_sections(description);
    let has_no_generated_context = matches!(
        sections.as_slice(),
        [domain::PullRequestDescriptionSection::Authored(text)] if *text == description
    );
    if has_no_generated_context {
        return render_authored_commit_markdown(
            &domain::pull_request_description_without_stack_context_markers(description),
            width,
        );
    }

    let mut rendered_sections = Vec::new();
    for section in sections {
        let rendered = match section {
            domain::PullRequestDescriptionSection::Authored(text) => {
                render_authored_commit_markdown(
                    &domain::pull_request_description_without_stack_context_markers(text),
                    width,
                )
            }
            domain::PullRequestDescriptionSection::GeneratedStackContext(text) => {
                render_generated_stack_context_for_terminal(text)
            }
        };
        push_non_empty_rendered_section(&mut rendered_sections, &rendered);
    }
    rendered_sections.join("\n\n")
}

fn render_authored_commit_markdown(description: &str, width: usize) -> String {
    MadSkin::default_light()
        .text(description, Some(width.max(20)))
        .to_string()
}

fn push_non_empty_rendered_section(sections: &mut Vec<String>, rendered: &str) {
    let rendered = trim_blank_lines(rendered);
    if !rendered.is_empty() {
        sections.push(rendered);
    }
}

fn render_generated_stack_context_for_terminal(context: &str) -> String {
    let mut rendered = Vec::new();
    let mut previous_blank = false;
    for line in context.lines() {
        let line = render_generated_stack_context_line(line);
        if line.trim().is_empty() {
            if !rendered.is_empty() && !previous_blank {
                rendered.push(String::new());
                previous_blank = true;
            }
            continue;
        }
        rendered.push(line);
        previous_blank = false;
    }
    while matches!(rendered.last(), Some(line) if line.is_empty()) {
        rendered.pop();
    }
    rendered.join("\n")
}

fn render_generated_stack_context_line(line: &str) -> String {
    let line = line.trim_end().replace("&nbsp;", " ");
    if let Some(heading) = line.trim().strip_prefix("### ") {
        return heading.to_owned();
    }
    render_stack_context_inline_markdown(&line)
}

/// Projects generated list emphasis and PR links while preserving escaped title text.
fn render_stack_context_inline_markdown(value: &str) -> String {
    let mut rendered = String::new();
    let mut offset = 0;
    while offset < value.len() {
        let remaining = &value[offset..];
        if let Some((consumed, ch)) = markdown_escaped_punctuation_prefix(remaining) {
            rendered.push(ch);
            offset += consumed;
            continue;
        }

        if let Some((content, consumed, style)) = parse_markdown_emphasis_prefix(remaining) {
            rendered.push_str(style);
            rendered.push_str(&render_stack_context_inline_markdown(content));
            rendered.push_str(RESET_STYLE);
            offset += consumed;
            continue;
        }

        if let Some(link) = parse_markdown_link_prefix(remaining) {
            rendered.push_str(&osc8_link(&link.url, &link.label));
            offset += link.consumed;
            continue;
        }

        let ch = remaining
            .chars()
            .next()
            .expect("non-empty remainder has a next character");
        rendered.push(ch);
        offset += ch.len_utf8();
    }
    rendered
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownLink {
    consumed: usize,
    label: String,
    url: String,
}

/// Reads the emphasis forms emitted for the current PR marker and lifecycle suffixes.
fn parse_markdown_emphasis_prefix(value: &str) -> Option<(&str, usize, &'static str)> {
    for (delimiter, style) in [("**", BOLD_STYLE), ("*", "\x1b[3m")] {
        let Some(inner) = value.strip_prefix(delimiter) else {
            continue;
        };
        // Older blocks put titles inside links; their literal asterisks do not close the outer emphasis.
        let search_start = parse_markdown_link_prefix(inner).map_or(0, |link| link.consumed);
        let Some(end) = inner[search_start..]
            .find(delimiter)
            .map(|end| search_start + end)
            .filter(|end| *end > 0)
        else {
            continue;
        };
        return Some((&inner[..end], end + 2 * delimiter.len(), style));
    }
    None
}

fn parse_markdown_link_prefix(value: &str) -> Option<MarkdownLink> {
    let label_end = markdown_link_label_end(value)?;
    let after_label = &value[label_end + 1..];
    let url = after_label.strip_prefix('(')?;
    let url_end = url.find(')')?;
    Some(MarkdownLink {
        consumed: label_end + 2 + url_end + 1,
        label: unescape_markdown_link_text(&value[1..label_end]),
        url: url[..url_end].to_owned(),
    })
}

fn markdown_escaped_punctuation_prefix(value: &str) -> Option<(usize, char)> {
    let rest = value.strip_prefix('\\')?;
    let ch = rest.chars().next()?;
    ch.is_ascii_punctuation().then_some((1 + ch.len_utf8(), ch))
}

fn markdown_link_label_end(value: &str) -> Option<usize> {
    if !value.starts_with('[') {
        return None;
    }

    let mut escaped = false;
    for (relative, ch) in value[1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            ']' => return Some(relative + 1),
            _ => {}
        }
    }
    None
}

fn unescape_markdown_link_text(value: &str) -> String {
    let mut rendered = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some(next) if next.is_ascii_punctuation() => rendered.push(next),
                Some(next) => {
                    rendered.push(ch);
                    rendered.push(next);
                }
                None => rendered.push(ch),
            }
        } else {
            rendered.push(ch);
        }
    }
    rendered
}

fn trim_blank_lines(value: &str) -> String {
    let lines = value.lines().collect::<Vec<_>>();
    let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .expect("a first non-empty line has a last non-empty line");
    lines[first..=last].join("\n")
}
