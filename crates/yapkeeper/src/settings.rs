//! What the user can set with `Ctrl+S`: each setting, its values in the
//! order the settings screen cycles them, their names and the defaults
//! `docs/interface.md` gives. How each is stored is the preferences'; how
//! each looks is the palette's.

/// What the user can set with `Ctrl+S`. `Default` is docs/interface.md's
/// defaults: on, on, on, off, Sage, Nerd Font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Leave out sessions in temp folders unless the query has `is:temp`.
    pub hide_temp: bool,
    /// Leave missing places out of the location list while nothing follows `@`.
    pub hide_missing: bool,
    /// List orphaned sessions after the rest.
    pub orphans_last: bool,
    /// Leave out orphaned sessions, and so missing places in the location
    /// list, unless the query has `is:orphaned`.
    pub hide_orphans: bool,
    pub accent: Accent,
    pub icons: IconSet,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            hide_temp: true,
            hide_missing: true,
            orphans_last: true,
            hide_orphans: false,
            accent: Accent::Sage,
            icons: IconSet::NerdFont,
        }
    }
}

/// The colour the palette draws its accent in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Sage,
    Cyan,
    Amber,
    Violet,
    Sky,
    Mono,
}

impl Accent {
    /// Every accent, in the order the settings screen cycles them.
    pub const ALL: [Accent; 6] = [
        Accent::Sage,
        Accent::Cyan,
        Accent::Amber,
        Accent::Violet,
        Accent::Sky,
        Accent::Mono,
    ];

    /// Such as "Sage".
    pub fn name(self) -> &'static str {
        match self {
            Accent::Sage => "Sage",
            Accent::Cyan => "Cyan",
            Accent::Amber => "Amber",
            Accent::Violet => "Violet",
            Accent::Sky => "Sky",
            Accent::Mono => "Mono",
        }
    }
}

/// The icons the palette shows for where a session ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSet {
    NerdFont,
    Unicode,
    TextTags,
    None,
}

impl IconSet {
    /// Every set, in the order the settings screen cycles them.
    pub const ALL: [IconSet; 4] = [
        IconSet::NerdFont,
        IconSet::Unicode,
        IconSet::TextTags,
        IconSet::None,
    ];

    /// "Nerd Font", "Unicode", "Text tags", "None".
    pub fn name(self) -> &'static str {
        match self {
            IconSet::NerdFont => "Nerd Font",
            IconSet::Unicode => "Unicode",
            IconSet::TextTags => "Text tags",
            IconSet::None => "None",
        }
    }
}
