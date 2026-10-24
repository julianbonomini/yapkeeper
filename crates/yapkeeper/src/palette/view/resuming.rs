//! The resume dialog, drawn in place of the query, the results and the
//! preview: which session and provider, then the directory and the exact
//! command resuming would use, each wrapped and never cut, then a word
//! labelled row for each state or warning that applies.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::Theme;
use crate::line::{availability_label, fit, one_line, without_controls, wrap};
use crate::palette::Palette;
use crate::palette::resuming::Resuming;
use crate::{Availability, Caution, ResumeCommand, ResumeDir};

/// Wide enough for the widest label, `unavailable`, and two spaces, so
/// every value starts in one column.
const LABEL_CELLS: usize = 13;

/// Draws the dialog's title into `top` and what resuming would do into
/// `body`.
pub(super) fn render(palette: &Palette, theme: &Theme, frame: &mut Frame, top: Rect, body: Rect) {
    let Some(resuming) = &palette.resuming else {
        return;
    };
    let session = &resuming.session;
    let width = usize::from(top.width);
    let provider = session.provider;
    // A space before the title and one before the provider.
    let quoted = width.saturating_sub(2 + "Resume \"\"".width() + provider.width());
    let title = format!("Resume \"{}\"", fit(&one_line(&session.title), quoted));
    let gap = width.saturating_sub(1 + title.width() + provider.width());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {title}"), theme.title),
            Span::raw(" ".repeat(gap)),
            Span::styled(provider, theme.dim),
        ])),
        top,
    );

    let plan = &resuming.plan;
    let cells = usize::from(body.width).saturating_sub(1 + LABEL_CELLS);
    let mut lines = Vec::new();
    let mut directory = plan.command.cwd.display().to_string();
    if let ResumeDir::Elsewhere(_) = plan.dir {
        directory.push_str(" (launch directory)");
    }
    lines.extend(labelled("directory", &directory, theme.dim, cells));
    lines.extend(labelled(
        "command",
        &command_line(&plan.command),
        theme.dim,
        cells,
    ));
    lines.push(Line::raw(""));

    // Below the directory and the command, so a short body cuts these first.
    for (label, text) in states(resuming, &palette.launch_dir) {
        lines.extend(labelled(label, &text, theme.label, cells));
    }
    frame.render_widget(Paragraph::new(lines), body);
}

/// A word label and its text for each state or warning that applies to
/// the dialog's plan, in the order they are drawn.
fn states(resuming: &Resuming, launch_dir: &Path) -> Vec<(&'static str, String)> {
    let plan = &resuming.plan;
    let mut states = Vec::new();
    let folder = match plan.recorded {
        Availability::Available => None,
        Availability::Orphaned => Some("The folder is gone, so the session can't resume there."),
        Availability::Unavailable => {
            Some("The folder exists but can't be inspected (permissions, or a volume not mounted).")
        }
    };
    if let Some(folder) = folder {
        let mut text = folder.to_string();
        if plan.offers_elsewhere() {
            text.push_str(&format!(" c resumes in {} instead.", launch_dir.display()));
        }
        if plan.offers_recheck() {
            text.push_str(" r checks again.");
        }
        states.push((availability_label(plan.recorded), text));
    }
    if resuming.checked_again {
        states.push(("", "Checked again: still can't inspect it.".to_string()));
    }
    if let ResumeDir::Elsewhere(_) = plan.dir {
        let launch = match plan.cwd {
            Availability::Available => None,
            Availability::Orphaned => Some("The launch directory is gone too."),
            Availability::Unavailable => Some("The launch directory can't be inspected either."),
        };
        if let Some(launch) = launch {
            states.push((availability_label(plan.cwd), launch.to_string()));
        }
    }
    let provider = resuming.session.provider;
    for caution in &plan.cautions {
        states.push(match caution {
            Caution::Running => (
                "running",
                format!("{provider} lists this session as open; resuming it again may clash."),
            ),
            Caution::MayNotFindElsewhere => (
                "warning",
                format!(
                    "{provider} looks sessions up by directory, so it may not find this one from another."
                ),
            ),
        });
    }
    if !plan.installed {
        let program = &plan.command.program;
        states.push((
            "not found",
            format!("{program} isn't on PATH, so it can't start."),
        ));
    }
    states
}

/// `label` in its column in `style`, then `text` wrapped into lines of
/// `cells` cells, each after the label's column; never cut.
fn labelled(label: &str, text: &str, style: Style, cells: usize) -> Vec<Line<'static>> {
    let text = without_controls(text);
    let wrapped = wrap(&text, cells, usize::MAX);
    wrapped
        .lines
        .into_iter()
        .enumerate()
        .map(|(n, range)| {
            let lead = match n {
                0 => format!(" {label:<width$}", width = LABEL_CELLS),
                _ => " ".repeat(1 + LABEL_CELLS),
            };
            Line::from(vec![
                Span::styled(lead, style),
                Span::raw(text[range].to_string()),
            ])
        })
        .collect()
}

/// `command` as one line a person could read or copy: the program, then
/// each argument, single-quoted when a shell would read it as more than
/// one word or as syntax. For display only; `launch` never uses it.
fn command_line(command: &ResumeCommand) -> String {
    let mut words = vec![shell_word(&command.program)];
    words.extend(command.args.iter().map(|arg| shell_word(arg)));
    without_controls(&words.join(" "))
}

/// `word` as a shell would read it back as that one word: as it is when
/// plain, otherwise single-quoted.
fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && !word
            .chars()
            .any(|c| c.is_whitespace() || "'\"$`\\!*?;&|<>()[]{}#~".contains(c));
    match plain {
        true => word.to_string(),
        false => format!("'{}'", word.replace('\'', "'\\''")),
    }
}
