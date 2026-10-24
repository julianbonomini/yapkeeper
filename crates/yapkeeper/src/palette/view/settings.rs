//! The settings screen, drawn in place of the query, the results and the
//! preview: one row per setting with its value, then where the index and
//! the preferences live.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{Theme, sample};
use crate::Settings;
use crate::line::{directory, fit, fit_left};
use crate::palette::settings::Setting;
use crate::palette::{Colour, Palette};

/// Wide enough for `preferences` and two spaces, so both paths line up.
const PATH_LABEL_CELLS: usize = 13;

/// Below this many body lines the blank line before the paths goes.
const ROOMY_BODY: u16 = 8;

/// Draws the screen's title into `top`, and its rows and paths into `body`.
pub(super) fn render(palette: &Palette, theme: &Theme, frame: &mut Frame, top: Rect, body: Rect) {
    let Some(screen) = &palette.settings else {
        return;
    };
    frame.render_widget(Paragraph::new(Line::styled(" Settings", theme.title)), top);
    let width = usize::from(body.width);
    let settings = palette.catalog.settings();
    let items: Vec<ListItem> = Setting::ALL
        .into_iter()
        .map(|setting| {
            let value = value(setting, settings, palette.colour);
            // A space before the label, at least one before the value, which
            // ends in the last cell and is never cut.
            let label = fit(label(setting), width.saturating_sub(2 + value.width()));
            let gap = width.saturating_sub(1 + label.width() + value.width());
            ListItem::new(Line::from(vec![
                Span::raw(format!(" {label}")),
                Span::raw(" ".repeat(gap)),
                Span::styled(value, theme.accent),
            ]))
        })
        .collect();
    let blank = u16::from(body.height >= ROOMY_BODY);
    let [rows, _, paths] = Layout::vertical([
        Constraint::Length(Setting::ALL.len() as u16),
        Constraint::Length(blank),
        Constraint::Length(2),
    ])
    .areas(body);
    let mut selected = ListState::default().with_selected(Some(screen.row()));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme.selection),
        rows,
        &mut selected,
    );

    let data = palette.catalog.data_dir();
    let home = palette.home.as_deref();
    let path_cells = width.saturating_sub(1 + PATH_LABEL_CELLS);
    let lines: Vec<Line> = [("index", data.index()), ("preferences", data.prefs())]
        .into_iter()
        .map(|(label, path)| {
            let path = fit_left(&directory(&path, home), path_cells);
            Line::styled(format!(" {label:<PATH_LABEL_CELLS$}{path}"), theme.dim)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), paths);
}

fn label(setting: Setting) -> &'static str {
    match setting {
        Setting::HideTemp => "Hide temp-folder sessions",
        Setting::HideMissing => "Hide missing folders in locations",
        Setting::OrphansLast => "List orphans last",
        Setting::Accent => "Accent",
        Setting::Icons => "Location icons",
    }
}

/// What `setting`'s row says it is set to in `settings`, saying so when
/// `colour` is `Mono` and the accent is therefore not drawn.
fn value(setting: Setting, settings: Settings, colour: Colour) -> String {
    let toggle = |on: bool| match on {
        true => "on".to_string(),
        false => "off".to_string(),
    };
    match setting {
        Setting::HideTemp => toggle(settings.hide_temp),
        Setting::HideMissing => toggle(settings.hide_missing),
        Setting::OrphansLast => toggle(settings.orphans_last),
        Setting::Accent => match colour {
            Colour::Accent => settings.accent.name().to_string(),
            Colour::Mono => format!("{} · off: NO_COLOR", settings.accent.name()),
        },
        // None has no sample, so nothing follows its name.
        Setting::Icons => format!("{} {}", settings.icons.name(), sample(settings.icons))
            .trim_end()
            .to_string(),
    }
}
