use super::*;
use super::{actions::DashboardActionInfo, status::DashboardStatus};
use std::{io::Write, time::Instant};

/// Paints notices outside the scrolling viewport and returns the height available for navigation.
pub(super) fn render_dashboard_frame(
    frame: Option<&PullRequestTableFrame>,
    size: DashboardTerminalSize,
    navigation: &mut DashboardNavigation,
    menu: Option<&mut PrActionMenu>,
    status: &DashboardStatus,
    running: Option<&DashboardActionInfo>,
) -> io::Result<DashboardTerminalSize> {
    let screen = dashboard_screen(
        frame,
        size,
        navigation,
        menu,
        status,
        running,
        Instant::now(),
    );
    write_dashboard_screen(&mut io::stdout(), &screen)?;
    Ok(screen.content_size)
}

fn dashboard_screen(
    frame: Option<&PullRequestTableFrame>,
    size: DashboardTerminalSize,
    navigation: &mut DashboardNavigation,
    menu: Option<&mut PrActionMenu>,
    status: &DashboardStatus,
    running: Option<&DashboardActionInfo>,
    now: Instant,
) -> DashboardScreen {
    let footer = (size.width > 0 && size.height > 0)
        .then(|| status.line(running, now, size.width))
        .flatten();
    let content_size = DashboardTerminalSize {
        width: size.width,
        height: size.height.saturating_sub(usize::from(footer.is_some())),
    };
    let (output, marker) = navigation.viewport(
        frame.map_or("", |frame| frame.text.as_str()),
        content_size.height,
    );
    let menu = menu.map(|menu| menu.screen(content_size, marker));
    DashboardScreen {
        size,
        content_size,
        output,
        marker,
        menu,
        footer,
    }
}

struct DashboardScreen {
    size: DashboardTerminalSize,
    content_size: DashboardTerminalSize,
    output: String,
    marker: Option<usize>,
    menu: Option<menu::MenuScreen>,
    footer: Option<String>,
}

fn write_dashboard_screen(output: &mut impl Write, screen: &DashboardScreen) -> io::Result<()> {
    queue!(
        output,
        terminal::BeginSynchronizedUpdate,
        MoveTo(0, 0),
        Clear(ClearType::All)
    )?;
    for (row, line) in clipped_dashboard_lines(&screen.output, screen.content_size)
        .into_iter()
        .enumerate()
    {
        queue!(output, MoveTo(0, row as u16))?;
        output.write_all(line.as_bytes())?;
    }
    if let Some(row) = screen.marker.filter(|_| screen.size.width > 0) {
        queue!(output, MoveTo(0, row as u16))?;
        // Paint only the existing left gutter; preserve the row's OSC8 links.
        output.write_all(b"\x1b[0;38;2;0;135;135m\xe2\x9d\xaf\x1b[0m")?;
    }
    if let Some(menu) = &screen.menu {
        for (row, line) in menu.lines.iter().enumerate() {
            queue!(output, MoveTo(menu.x as u16, (menu.y + row) as u16))?;
            output.write_all(line.as_bytes())?;
        }
    }
    if let Some(footer) = &screen.footer {
        queue!(
            output,
            terminal::DisableLineWrap,
            MoveTo(0, (screen.size.height - 1) as u16)
        )?;
        let written = output.write_all(footer.as_bytes());
        let restored = queue!(output, terminal::EnableLineWrap);
        written?;
        restored?;
    }
    queue!(output, terminal::EndSynchronizedUpdate)?;
    output.flush()
}

fn clipped_dashboard_lines(output: &str, terminal_size: DashboardTerminalSize) -> Vec<String> {
    output
        .split('\n')
        .take(terminal_size.height)
        .map(|line| ellipsize_rendered_line(line.trim_end_matches('\r'), Some(terminal_size.width)))
        .collect()
}

#[cfg(test)]
#[path = "tests/screen.rs"]
mod tests;
