//! The resume dialog's state: the session it opened on and the plan drawn
//! for it now. What the plan says is the catalog's; this module only
//! chooses which plan to ask for.

use std::path::Path;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{Availability, Catalog, ResumeCommand, ResumeDir, ResumePlan, Session};

/// The resume dialog: the session it opened on and the plan drawn now.
pub(super) struct Resuming {
    pub(super) session: Session,
    pub(super) plan: ResumePlan,
    /// `r` found the session's own directory still unavailable.
    pub(super) checked_again: bool,
}

impl Resuming {
    /// The plan for `session` in its own directory.
    pub(super) fn open(catalog: &Catalog, session: &Session) -> Resuming {
        Resuming {
            session: session.clone(),
            plan: catalog.resume(session, ResumeDir::Recorded),
            checked_again: false,
        }
    }

    /// `Enter` gives the command when the plan can start; `c` plans in
    /// `launch_dir` while the plan offers it; `r` plans again in the same
    /// directory while it offers it. Other keys do nothing. `Esc`
    /// and `Ctrl+C` are the palette's.
    pub(super) fn handle(
        &mut self,
        key: KeyEvent,
        catalog: &Catalog,
        launch_dir: &Path,
    ) -> Option<ResumeCommand> {
        let plain = (key.modifiers - KeyModifiers::SHIFT).is_empty();
        match key.code {
            KeyCode::Enter if self.plan.can_start() => return Some(self.plan.command.clone()),
            KeyCode::Char('c') if plain && self.plan.offers_elsewhere() => {
                let elsewhere = ResumeDir::Elsewhere(launch_dir.to_path_buf());
                self.plan = catalog.resume(&self.session, elsewhere);
                self.checked_again = false;
            }
            KeyCode::Char('r') if plain && self.plan.offers_recheck() => {
                self.plan = catalog.resume(&self.session, self.plan.dir.clone());
                self.checked_again = self.plan.recorded == Availability::Unavailable;
            }
            _ => {}
        }
        None
    }
}
