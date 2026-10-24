//! Where the person is on the settings screen: which setting is selected,
//! and what each key does to the settings. Saving them is the catalog's.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{Accent, IconSet, Settings};

/// One row of the settings screen: a setting the person can change.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Setting {
    HideTemp,
    HideMissing,
    OrphansLast,
    Accent,
    Icons,
}

impl Setting {
    /// Every setting, in the order the screen lists them.
    pub(super) const ALL: [Setting; 5] = [
        Setting::HideTemp,
        Setting::HideMissing,
        Setting::OrphansLast,
        Setting::Accent,
        Setting::Icons,
    ];
}

/// The settings screen's selected row.
pub(super) struct SettingsScreen {
    row: usize,
}

impl SettingsScreen {
    /// On the first setting.
    pub(super) fn new() -> SettingsScreen {
        SettingsScreen { row: 0 }
    }

    /// Moves among the settings with `↑` `↓`, `Ctrl+P` and `Ctrl+N`,
    /// stopping at the first and last. `→` and `Enter` change the selected
    /// setting to its next value, `←` to its previous: a toggle flips, and
    /// the accent and the icon set cycle, wrapping around. Returns
    /// `settings` as the key changes them, or `None` when it changes none.
    /// Other keys do nothing.
    pub(super) fn handle(&mut self, key: KeyEvent, settings: Settings) -> Option<Settings> {
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        let by = match key.code {
            KeyCode::Up => return self.move_to(self.row.saturating_sub(1)),
            KeyCode::Char('p') if ctrl => return self.move_to(self.row.saturating_sub(1)),
            KeyCode::Down => return self.move_to(self.row + 1),
            KeyCode::Char('n') if ctrl => return self.move_to(self.row + 1),
            KeyCode::Right | KeyCode::Enter => 1,
            KeyCode::Left => -1,
            _ => return None,
        };
        let mut changed = settings;
        match Setting::ALL[self.row] {
            Setting::HideTemp => changed.hide_temp = !settings.hide_temp,
            Setting::HideMissing => changed.hide_missing = !settings.hide_missing,
            Setting::OrphansLast => changed.orphans_last = !settings.orphans_last,
            Setting::Accent => changed.accent = cycle(&Accent::ALL, settings.accent, by),
            Setting::Icons => changed.icons = cycle(&IconSet::ALL, settings.icons, by),
        }
        Some(changed)
    }

    /// Selects `row`, or the last setting when there are fewer; changes none.
    fn move_to(&mut self, row: usize) -> Option<Settings> {
        self.row = row.min(Setting::ALL.len() - 1);
        None
    }

    /// Where the selected setting is in `Setting::ALL`.
    pub(super) fn row(&self) -> usize {
        self.row
    }
}

/// The value `by` steps from `current` in `all`, wrapping around.
fn cycle<T: Copy + PartialEq>(all: &[T], current: T, by: isize) -> T {
    let at = all.iter().position(|value| *value == current).unwrap_or(0);
    all[(at as isize + by).rem_euclid(all.len() as isize) as usize]
}
