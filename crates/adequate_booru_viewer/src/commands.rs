use std::sync::OnceLock;

use eternalist_apps::{
    command_guide::{GuideGesture, GuideSection, PANEL_IDIOMS, RAIL_IDIOMS},
    commands::{CommandCanon, CommandScope, CommandSpec, Shortcut, ShortcutKey, ShortcutModifiers},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edict {
    FocusTagEntry,
    NextQueryGroup,
    PreviousQueryGroup,
    ToggleViewerTags,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Context {
    Workbench,
    Viewer,
}

const PREVIOUS_GROUP: [Shortcut; 1] = [Shortcut::new(
    ShortcutModifiers::ALT.plus(ShortcutModifiers::SHIFT),
    ShortcutKey::Character('G'),
)];
const TOGGLE_TAGS: [Shortcut; 1] = [Shortcut::new(
    ShortcutModifiers::NONE,
    ShortcutKey::Character('T'),
)];

const EDICTS: [CommandSpec<Edict, Context>; 4] = [
    CommandSpec::new(
        Edict::FocusTagEntry,
        "query.focus_tag_entry",
        "Focus tag entry",
        CommandScope::Context(Context::Workbench),
    )
    .with_detail("Moves keyboard focus to the active query group's tag entry.")
    .with_mnemonic('F'),
    CommandSpec::new(
        Edict::NextQueryGroup,
        "query.next_group",
        "Next query group",
        CommandScope::Context(Context::Workbench),
    )
    .with_detail("Moves the highlighted insertion point to the next Boolean group.")
    .with_mnemonic('G'),
    CommandSpec::new(
        Edict::PreviousQueryGroup,
        "query.previous_group",
        "Previous query group",
        CommandScope::Context(Context::Workbench),
    )
    .with_detail("Moves the highlighted insertion point to the previous Boolean group.")
    .with_default_shortcuts(&PREVIOUS_GROUP),
    CommandSpec::new(
        Edict::ToggleViewerTags,
        "viewer.toggle_tags",
        "Tags",
        CommandScope::Context(Context::Viewer),
    )
    .with_detail("Shows or hides the open image's tag drawer.")
    .with_default_shortcuts(&TOGGLE_TAGS)
    .with_mnemonic('T'),
];

const COMPLETION_KEYS: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Tab),
    Shortcut::new(ShortcutModifiers::SHIFT, ShortcutKey::Tab),
];
const ENTER: [Shortcut; 1] = [Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Enter)];
const VIEWER_ARROWS: [Shortcut; 4] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowLeft),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowUp),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowDown),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowRight),
];
const RESULT_ARROWS: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::SHIFT, ShortcutKey::ArrowLeft),
    Shortcut::new(ShortcutModifiers::SHIFT, ShortcutKey::ArrowRight),
];
const ESCAPE: [Shortcut; 1] = [Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Escape)];

const QUERY_GESTURES: [GuideGesture; 3] = [
    GuideGesture::new(
        "Focus tag entry",
        "Slash moves focus here when no text editor already owns typing.",
        &[Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Slash)],
    ),
    GuideGesture::new(
        "Choose completion",
        "Cycles forward or backward through suggestions while the tag entry owns focus.",
        &COMPLETION_KEYS,
    ),
    GuideGesture::new(
        "Add terms",
        "Adds the typed tags to the highlighted group; prefix a tag with minus to exclude it.",
        &ENTER,
    ),
];
const GALLERY_GESTURES: [GuideGesture; 3] = [
    GuideGesture::new(
        "Open image",
        "Click a thumbnail to enter the full viewer.",
        &[],
    ),
    GuideGesture::new(
        "Inspect thumbnail tags",
        "Right-click a thumbnail to open its tag palette.",
        &[],
    ),
    GuideGesture::new(
        "Adjust gallery density",
        "Use Control+wheel over the gallery, or focus the images-per-row rail and use arrows.",
        &[],
    ),
];
const VIEWER_GESTURES: [GuideGesture; 6] = [
    GuideGesture::new(
        "Navigate family",
        "Moves to a sibling, parent, or first child; in the family tree it moves selection.",
        &VIEWER_ARROWS,
    ),
    GuideGesture::new(
        "Navigate results",
        "Moves to the previous or next result regardless of family structure.",
        &RESULT_ARROWS,
    ),
    GuideGesture::new(
        "Open selected image",
        "Returns from the family tree to its selected image.",
        &ENTER,
    ),
    GuideGesture::new(
        "Open family tree",
        "Right-click the image or wheel downward over it.",
        &[],
    ),
    GuideGesture::new(
        "Move around family tree",
        "Drag to pan and use the wheel to zoom.",
        &[],
    ),
    GuideGesture::new(
        "Close viewer",
        "Returns to the gallery without disturbing the active query.",
        &ESCAPE,
    ),
];

const QUERY_IDIOMS: GuideSection = GuideSection::new("REFERENCE QUERY", &QUERY_GESTURES);
const GALLERY_IDIOMS: GuideSection = GuideSection::new("GALLERY", &GALLERY_GESTURES);
const VIEWER_IDIOMS: GuideSection = GuideSection::new("IMAGE VIEWER", &VIEWER_GESTURES);

pub const WORKBENCH_IDIOMS: [GuideSection; 4] =
    [PANEL_IDIOMS, RAIL_IDIOMS, QUERY_IDIOMS, GALLERY_IDIOMS];
pub const VIEWER_CONTEXT_IDIOMS: [GuideSection; 1] = [VIEWER_IDIOMS];

pub fn canon() -> &'static CommandCanon<Edict, Context> {
    static CANON: OnceLock<CommandCanon<Edict, Context>> = OnceLock::new();
    CANON.get_or_init(|| CommandCanon::new(&EDICTS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edict_canon_is_valid() {
        assert_eq!(canon().specs().len(), EDICTS.len());
    }
}
