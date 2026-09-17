use crate::domain::PrActionContext;

/// Terminal text and PR targets emitted together; OSC8 links remain part of the untouched text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::commands) struct PullRequestTableFrame {
    pub(in crate::commands) text: String,
    pub(in crate::commands) rows: Vec<RenderedPrRow>,
    line_count: usize,
}

/// A PR's first output line and full action context, independent of its abbreviated display cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct RenderedPrRow {
    pub(in crate::commands) line: usize,
    pub(in crate::commands) context: PrActionContext,
}

impl PullRequestTableFrame {
    /// Appends an unselectable line without modifying its terminal escape sequences.
    pub(in crate::commands) fn push_line(&mut self, text: &str) {
        self.text.push_str(text);
        self.text.push('\n');
        self.line_count += 1 + text.bytes().filter(|byte| *byte == b'\n').count();
    }

    /// Records the target while emitting its row; branch-only rows have no PR target.
    pub(in crate::commands) fn push_pr_line(
        &mut self,
        text: &str,
        context: Option<PrActionContext>,
    ) {
        if let Some(context) = context {
            self.rows.push(RenderedPrRow {
                line: self.line_count,
                context,
            });
        }
        self.push_line(text);
    }
}

#[cfg(test)]
#[path = "tests/pr_table.rs"]
mod tests;
