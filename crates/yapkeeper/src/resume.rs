//! What resuming a session would do, and doing it: whether the directory
//! it would start in can be used now, whether the provider's program is on
//! a `PATH`, and which warnings apply, from the disk and the provider's
//! metadata. The provider's own process is started here, with no shell.
//! The arguments are the adapter's; this module never builds them.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use yapkeeper_model::{Adapter, RecordedGit, ResumeCommand, Session};

use crate::{Availability, Location};

/// Where to start the provider: the session's own working directory, or
/// another one the person chose (the launch directory, after `c`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDir {
    Recorded,
    Elsewhere(PathBuf),
}

/// What resuming one session in one directory would do, as the disk and
/// `PATH` showed it when the plan was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePlan {
    /// The adapter's exact process; `cwd` is where it would start.
    pub command: ResumeCommand,
    pub dir: ResumeDir,
    /// The session's own working directory.
    pub recorded: Availability,
    /// `command.cwd`; the same as `recorded` while `dir` is `Recorded`.
    pub cwd: Availability,
    /// Whether `command.program` was found on the catalog's `PATH`.
    pub installed: bool,
    /// Warnings that don't stop it, in this order.
    pub cautions: Vec<Caution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caution {
    /// The provider's metadata lists the session as open now.
    Running,
    /// `dir` is `Elsewhere` and the provider finds sessions by directory.
    MayNotFindElsewhere,
}

impl ResumePlan {
    /// Whether `launch` would start it: the program is installed and
    /// `cwd` is available.
    pub fn can_start(&self) -> bool {
        self.installed && self.cwd == Availability::Available
    }

    /// Whether resuming in another directory is on offer (`c`): the
    /// session's own directory is orphaned or unavailable and this plan
    /// still starts there.
    pub fn offers_elsewhere(&self) -> bool {
        self.dir == ResumeDir::Recorded && self.recorded != Availability::Available
    }

    /// Whether checking the session's own directory again could change
    /// this plan (`r`): it is unavailable.
    pub fn offers_recheck(&self) -> bool {
        self.recorded == Availability::Unavailable
    }
}

/// What resuming `session` with `adapter` in `dir` would do, read from
/// the disk and `path` now. Never fails.
pub(crate) fn plan(
    adapter: &dyn Adapter,
    session: &Session,
    dir: ResumeDir,
    path: &OsStr,
) -> ResumePlan {
    let cwd = match &dir {
        ResumeDir::Recorded => &session.working_dir,
        ResumeDir::Elsewhere(elsewhere) => elsewhere,
    };
    let mut cautions = Vec::new();
    if adapter.is_running(&session.id) {
        cautions.push(Caution::Running);
    }
    if matches!(dir, ResumeDir::Elsewhere(_)) && adapter.resumes_by_directory() {
        cautions.push(Caution::MayNotFindElsewhere);
    }
    let command = adapter.resume(&session.id, cwd);
    ResumePlan {
        installed: on_path(&command.program, path),
        command,
        recorded: availability(&session.working_dir),
        cwd: availability(cwd),
        dir,
        cautions,
    }
}

/// Whether `program` names an executable file, itself when it holds a `/`,
/// otherwise in one of the directories of `path`, as a shell would find it.
fn on_path(program: &str, path: &OsStr) -> bool {
    if program.contains('/') {
        return executable(Path::new(program));
    }
    std::env::split_paths(path).any(|dir| executable(&dir.join(program)))
}

fn executable(file: &Path) -> bool {
    let Ok(metadata) = fs::metadata(file) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// Whether `dir` can be started in now, judged as a session's location is.
fn availability(dir: &Path) -> Availability {
    Location::of(dir, &RecordedGit::default()).availability
}

/// Replaces this process with `command`: its program, its args as
/// separate arguments with no shell, in `command.cwd`. Returns only when
/// it couldn't, with why. On a system without `exec`, runs it, waits and
/// exits with its code. Call it only once the terminal is restored.
pub fn launch(command: &ResumeCommand) -> io::Error {
    let mut process = Command::new(&command.program);
    process.args(&command.args).current_dir(&command.cwd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.exec()
    }
    #[cfg(not(unix))]
    match process.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => error,
    }
}
