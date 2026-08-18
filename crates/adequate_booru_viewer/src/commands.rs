use std::sync::OnceLock;

use eternalist_apps::{
    command_guide::{GuideGesture, GuideSection},
    commands::{CommandCanon, CommandScope, CommandSpec, Shortcut, ShortcutKey, ShortcutModifiers},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edict {
    FocusTagEntry,
    NextQueryGroup,
    PreviousQueryGroup,
    OpenViewerTree,
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
const OPEN_VIEWER_TREE: [Shortcut; 1] = [Shortcut::new(
    ShortcutModifiers::NONE,
    ShortcutKey::Character('R'),
)];
const TOGGLE_VIEWER_TAGS: [Shortcut; 1] = [Shortcut::new(
    ShortcutModifiers::NONE,
    ShortcutKey::Character('T'),
)];

const EDICTS: [CommandSpec<Edict, Context>; 5] = [
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
        Edict::OpenViewerTree,
        "viewer.open_tree",
        "View tree",
        CommandScope::Context(Context::Viewer),
    )
    .with_detail(
        "Opens the current image's family tree; right-clicking or wheeling down over the image does the same.",
    )
    .with_default_shortcuts(&OPEN_VIEWER_TREE),
    CommandSpec::new(
        Edict::ToggleViewerTags,
        "viewer.toggle_tags",
        "Tags",
        CommandScope::Context(Context::Viewer),
    )
    .with_detail("Shows or hides the open image's tag drawer.")
    .with_default_shortcuts(&TOGGLE_VIEWER_TAGS),
];

const COMPLETION_KEYS: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Tab),
    Shortcut::new(ShortcutModifiers::SHIFT, ShortcutKey::Tab),
];
const ENTER: [Shortcut; 1] = [Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Enter)];
const RETURN_TO_IMAGE: [Shortcut; 3] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Character('R')),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Enter),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Escape),
];
const FAMILY_ARROWS: [Shortcut; 4] = [
    Shortcut::new(ShortcutModifiers::ALT, ShortcutKey::ArrowLeft),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowUp),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowDown),
    Shortcut::new(ShortcutModifiers::ALT, ShortcutKey::ArrowRight),
];
const TREE_ARROWS: [Shortcut; 4] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowLeft),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowUp),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowDown),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowRight),
];
const RESULT_ARROWS: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowLeft),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowRight),
];
const ALTERNATE_RESULT_ARROWS: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::ALT, ShortcutKey::ArrowLeft),
    Shortcut::new(ShortcutModifiers::ALT, ShortcutKey::ArrowRight),
];
const RESULT_ROWS: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::PageUp),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::PageDown),
];
const RESULT_HOME: [Shortcut; 1] = [Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Home)];
const ESCAPE: [Shortcut; 1] = [Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Escape)];
const TOGGLE_SIDEBAR: [Shortcut; 1] = [Shortcut::new(
    ShortcutModifiers::NONE,
    ShortcutKey::Function(9),
)];
const NEXT_SIDEBAR_SECTION: [Shortcut; 1] =
    [Shortcut::new(ShortcutModifiers::CONTROL, ShortcutKey::Tab)];
const PREVIOUS_SIDEBAR_SECTION: [Shortcut; 1] = [Shortcut::new(
    ShortcutModifiers::CONTROL.plus(ShortcutModifiers::SHIFT),
    ShortcutKey::Tab,
)];
const ADJUST_VALUE: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowLeft),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::ArrowRight),
];
const VALUE_BOUNDS: [Shortcut; 2] = [
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::Home),
    Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::End),
];

const SIDEBAR_GESTURES: [GuideGesture; 5] = [
    GuideGesture::new(
        "Show or hide sidebar",
        "Conceals or reveals the filter and gallery controls.",
        &TOGGLE_SIDEBAR,
    ),
    GuideGesture::new(
        "Next sidebar section",
        "Moves focus to the next section of the sidebar.",
        &NEXT_SIDEBAR_SECTION,
    ),
    GuideGesture::new(
        "Previous sidebar section",
        "Moves focus to the previous section of the sidebar.",
        &PREVIOUS_SIDEBAR_SECTION,
    ),
    GuideGesture::new(
        "Adjust value",
        "Changes a focused value by one step; hovered values also accept the wheel.",
        &ADJUST_VALUE,
    ),
    GuideGesture::new(
        "Minimum or maximum",
        "Moves a focused value directly to its minimum or maximum.",
        &VALUE_BOUNDS,
    ),
];
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
const GALLERY_GESTURES: [GuideGesture; 5] = [
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
        "Use Control+wheel over the gallery, or focus the images-per-row control and use arrows.",
        &[],
    ),
    GuideGesture::new(
        "Navigate result rows",
        "Scrolls exactly one rendered gallery row.",
        &RESULT_ROWS,
    ),
    GuideGesture::new(
        "First result",
        "Returns the gallery to its first row.",
        &RESULT_HOME,
    ),
];
const IMAGE_NAVIGATION_GESTURES: [GuideGesture; 3] = [
    GuideGesture::new(
        "Navigate results",
        "Moves one item through the global result list, including while viewing a family member.",
        &RESULT_ARROWS,
    ),
    GuideGesture::new(
        "Navigate family",
        "Alt+Left/Right moves across the current family level; Up/Down moves to the parent or first child.",
        &FAMILY_ARROWS,
    ),
    GuideGesture::new(
        "Close viewer",
        "Returns to the gallery without disturbing the active query.",
        &ESCAPE,
    ),
];
const FAMILY_TREE_GESTURES: [GuideGesture; 4] = [
    GuideGesture::new(
        "Navigate tree",
        "Left/Right moves across the current family level; Up/Down moves to the parent or first child.",
        &TREE_ARROWS,
    ),
    GuideGesture::new(
        "Navigate results",
        "Moves one item through the global result list and returns to the image viewer.",
        &ALTERNATE_RESULT_ARROWS,
    ),
    GuideGesture::new(
        "Return to selected image",
        "Returns from the family tree to its selected image.",
        &RETURN_TO_IMAGE,
    ),
    GuideGesture::new(
        "Move around family tree",
        "Drag to pan and use the wheel to zoom.",
        &[],
    ),
];

const SIDEBAR_IDIOMS: GuideSection = GuideSection::new("SIDEBAR", &SIDEBAR_GESTURES);
const QUERY_IDIOMS: GuideSection = GuideSection::new("REFERENCE QUERY", &QUERY_GESTURES);
const GALLERY_IDIOMS: GuideSection = GuideSection::new("GALLERY", &GALLERY_GESTURES);
const IMAGE_NAVIGATION_IDIOMS: GuideSection =
    GuideSection::new("NAVIGATION", &IMAGE_NAVIGATION_GESTURES);
const FAMILY_TREE_IDIOMS: GuideSection = GuideSection::new("FAMILY TREE", &FAMILY_TREE_GESTURES);

pub const WORKBENCH_IDIOMS: [GuideSection; 3] = [SIDEBAR_IDIOMS, QUERY_IDIOMS, GALLERY_IDIOMS];
pub const IMAGE_VIEWER_IDIOMS: [GuideSection; 1] = [IMAGE_NAVIGATION_IDIOMS];
pub const FAMILY_VIEWER_IDIOMS: [GuideSection; 1] = [FAMILY_TREE_IDIOMS];

pub fn canon() -> &'static CommandCanon<Edict, Context> {
    static CANON: OnceLock<CommandCanon<Edict, Context>> = OnceLock::new();
    CANON.get_or_init(|| CommandCanon::new(&EDICTS))
}
