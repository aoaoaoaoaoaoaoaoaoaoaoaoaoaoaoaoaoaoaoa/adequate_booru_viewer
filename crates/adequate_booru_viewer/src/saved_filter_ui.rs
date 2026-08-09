use crate::{
    chrome,
    config::{FilterName, FilterSelection, SavedFilter},
    controls,
    filter_bank::{Bank, Berth, ShelfBerth},
    water,
};
use eternalist_apps::CabinetAction;

#[derive(Clone, Debug)]
pub enum Action {
    New,
    Save,
    BeginNameEdit,
    Rename,
    Load(SavedFilter),
    LoadLocalFavorites,
    Clone(FilterName),
    Delete(FilterName),
    RenameEntry { from: FilterName, to: FilterName },
    Moor { name: FilterName, berth: Berth },
    MoorShelf { shelf: usize, berth: ShelfBerth },
    NewShelf,
    ToggleShelf(usize),
    ScuttleShelf(usize),
    BeginShelfRename(usize),
    CommitShelfRename,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NameEdit {
    #[default]
    Idle,
    Arming,
    Editing,
}

pub use eternalist_apps::CabinetShelfEdit as ShelfEdit;
pub type EntryEdit = eternalist_apps::CabinetEntryEdit<FilterName>;

pub fn active_card(
    ui: &mut egui::Ui,
    water: &mut water::Surface,
    name_entry: &mut String,
    edit: &mut NameEdit,
    selection: &FilterSelection,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let _title = ui.horizontal(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if *selection != FilterSelection::LocalFavorites
            && controls::symbol(ui, water, chrome::Symbol::Rename)
                .on_hover_text("rename in place")
                .clicked()
        {
            actions.push(Action::BeginNameEdit);
        }
        if *edit == NameEdit::Idle {
            let _name = ui.label(match selection {
                FilterSelection::Scratch => chrome::title("new unsaved filter"),
                FilterSelection::Saved { name } => chrome::title(name.to_string()),
                FilterSelection::LocalFavorites => chrome::title("♥ favorites"),
            });
        } else {
            let before = name_entry.clone();
            let entry = ui.add_sized(
                [ui.available_width(), 20.0],
                egui::TextEdit::singleline(name_entry).hint_text("filter name"),
            );
            if let Some(wake) = chrome::text_wake(ui, &entry, &before, name_entry) {
                water.text(wake);
            }
            if *edit == NameEdit::Arming {
                entry.request_focus();
                *edit = NameEdit::Editing;
            }
            let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
            if enter && (entry.has_focus() || entry.lost_focus()) {
                actions.push(match selection {
                    FilterSelection::Saved { .. } => Action::Rename,
                    FilterSelection::Scratch => Action::Save,
                    FilterSelection::LocalFavorites => {
                        unreachable!("local favorites cannot enter name editing")
                    }
                });
            } else if entry.lost_focus() {
                *edit = NameEdit::Idle;
            }
        }
    });
    let _save = ui.horizontal_wrapped(|ui| {
        if controls::symbol(ui, water, chrome::Symbol::Add)
            .on_hover_text("new filter")
            .clicked()
        {
            actions.push(Action::New);
        }
        match selection {
            FilterSelection::Saved { name: active } => {
                let assembly = chrome::Coupled::horizontal_with_gap(
                    ui,
                    chrome::CouplingGap::MINIMUM,
                    |ui| {
                        chrome::Monoglyph::symbol(chrome::Symbol::Confirm)
                            .show(ui)
                            .on_hover_text("save")
                    },
                    |ui| {
                        chrome::Monoglyph::symbol(chrome::Symbol::Duplicate)
                            .show(ui)
                            .on_hover_text("clone")
                    },
                );
                water.monoglyph(&assembly.left);
                water.monoglyph(&assembly.right);
                if assembly.left.clicked() {
                    actions.push(if *edit == NameEdit::Idle {
                        Action::Save
                    } else {
                        Action::Rename
                    });
                }
                if assembly.right.clicked() {
                    actions.push(Action::Clone(active.clone()));
                }
            }
            FilterSelection::Scratch => {
                if controls::symbol(ui, water, chrome::Symbol::Confirm)
                    .on_hover_text("save")
                    .clicked()
                {
                    actions.push(Action::Save);
                }
            }
            FilterSelection::LocalFavorites => {}
        }
    });
    actions
}

pub fn library(
    ui: &mut egui::Ui,
    water: &mut water::Surface,
    selection: &FilterSelection,
    bank: &Bank,
    shelf_edit: &mut Option<ShelfEdit>,
    entry_edit: &mut Option<EntryEdit>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    local_favorites_row(
        ui,
        water,
        *selection == FilterSelection::LocalFavorites,
        &mut actions,
    );
    actions.extend(
        bank.show_renamable(
            ui,
            water,
            "filters",
            "filter",
            selection.saved(),
            shelf_edit,
            entry_edit,
        )
        .into_iter()
        .map(Action::from),
    );
    actions
}

fn local_favorites_row(
    ui: &mut egui::Ui,
    water: &mut water::Surface,
    selected: bool,
    actions: &mut Vec<Action>,
) {
    let text = if selected {
        "● ♥ favorites"
    } else {
        "♥ favorites"
    };
    let response = ui
        .selectable_label(selected, text)
        .on_hover_text("built-in: show every locally favorited image");
    crate::probe_anchor!(ui, "filter:local-favorites", response.interact_rect);
    if chrome::hover_started(ui, &response) {
        water.bump(response.rect);
    }
    if response.clicked() {
        actions.push(Action::LoadLocalFavorites);
    }
}

impl From<CabinetAction<SavedFilter>> for Action {
    fn from(action: CabinetAction<SavedFilter>) -> Self {
        match action {
            CabinetAction::Load(filter) => Self::Load(filter),
            CabinetAction::Clone(name) => Self::Clone(name),
            CabinetAction::Delete(name) => Self::Delete(name),
            CabinetAction::RenameEntry { from, to } => Self::RenameEntry { from, to },
            CabinetAction::Moor { key: name, berth } => Self::Moor { name, berth },
            CabinetAction::MoorShelf { shelf, berth } => Self::MoorShelf { shelf, berth },
            CabinetAction::NewShelf => Self::NewShelf,
            CabinetAction::ToggleShelf(shelf) => Self::ToggleShelf(shelf),
            CabinetAction::ScuttleShelf(shelf) => Self::ScuttleShelf(shelf),
            CabinetAction::BeginShelfRename(shelf) => Self::BeginShelfRename(shelf),
            CabinetAction::CommitShelfRename => Self::CommitShelfRename,
        }
    }
}
