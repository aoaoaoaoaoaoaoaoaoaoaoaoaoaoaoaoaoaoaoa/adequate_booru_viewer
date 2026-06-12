use crate::{
    chrome,
    config::{FilterName, SavedFilter, Shelf},
    filter_bank::{Bank, Berth},
};

#[derive(Clone, Debug)]
pub enum Action {
    New,
    Save,
    BeginNameEdit,
    Rename,
    Load(SavedFilter),
    Clone(FilterName),
    Delete(FilterName),
    Moor { name: FilterName, berth: Berth },
    NewShelf,
    ToggleShelf(usize),
    TypeWake(chrome::TextWake),
    Pulse(egui::Rect),
    ScuttleShelf(usize),
    BeginShelfRename(usize),
    CommitShelfRename,
}

/// The active-filter name edit: armed (focus pending), live, or idle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NameEdit {
    #[default]
    Idle,
    Arming,
    Editing,
}

/// In-flight folder rename, owned by the app between frames.
#[derive(Clone, Debug)]
pub struct ShelfEdit {
    pub shelf: usize,
    pub name: String,
    pub focus: bool,
}

pub fn active_card(
    ui: &mut egui::Ui,
    name_entry: &mut String,
    edit: &mut NameEdit,
    active: Option<&FilterName>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let _title = ui.horizontal(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if chrome::icon(ui, "✎")
            .on_hover_text("rename in place")
            .clicked()
        {
            actions.push(Action::BeginNameEdit);
        }
        if *edit == NameEdit::Idle {
            let _name = ui.label(match active {
                Some(name) => chrome::title(name.to_string()),
                None => chrome::title("new unsaved filter"),
            });
        } else {
            // The pencil edits the name where it is written.
            let before = name_entry.clone();
            let entry = ui.add_sized(
                [ui.available_width(), 20.0],
                egui::TextEdit::singleline(name_entry).hint_text("filter name"),
            );
            if let Some(wake) = chrome::text_wake(ui, &entry, &before, name_entry) {
                actions.push(Action::TypeWake(wake));
            }
            if *edit == NameEdit::Arming {
                entry.request_focus();
                *edit = NameEdit::Editing;
            }
            let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
            if enter && (entry.has_focus() || entry.lost_focus()) {
                actions.push(if active.is_some() {
                    Action::Rename
                } else {
                    Action::Save
                });
            } else if entry.lost_focus() {
                *edit = NameEdit::Idle;
            }
        }
    });
    let _save = ui.horizontal_wrapped(|ui| {
        if chrome::icon(ui, "✚").on_hover_text("new filter").clicked() {
            actions.push(Action::New);
        }
        if chrome::icon(ui, "✓").on_hover_text("save").clicked() {
            actions.push(if *edit != NameEdit::Idle && active.is_some() {
                Action::Rename
            } else {
                Action::Save
            });
        }
        if let Some(active) = active
            && chrome::icon(ui, "⧉").on_hover_text("clone").clicked()
        {
            actions.push(Action::Clone(active.clone()));
        }
    });
    actions
}

pub fn library(
    ui: &mut egui::Ui,
    active: Option<&FilterName>,
    bank: &Bank,
    shelf_edit: &mut Option<ShelfEdit>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if bank.root.is_empty() && bank.shelves.is_empty() {
        let _empty = ui.label("none");
    }
    for filter in &bank.root {
        filter_row(ui, active, filter, &mut actions);
    }
    for (slot, shelf) in bank.shelves.iter().enumerate() {
        shelf_rows(ui, slot, shelf, active, shelf_edit, &mut actions);
    }
    let _controls = ui.horizontal_wrapped(|ui| {
        if chrome::icon(ui, "⊞").on_hover_text("new folder").clicked() {
            actions.push(Action::NewShelf);
        }
    });
    root_basin(ui, &mut actions);
    actions
}

fn filter_row(
    ui: &mut egui::Ui,
    active: Option<&FilterName>,
    filter: &SavedFilter,
    actions: &mut Vec<Action>,
) {
    let row = ui.horizontal(|ui| {
        // The truncating name label must come last, so every control stays
        // inside the panel no matter how long the name runs.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let drag_id = egui::Id::new(("filter-drag", filter.name.as_str()));
        let _handle = ui.dnd_drag_source(drag_id, filter.name.clone(), |ui| {
            let _grip = ui
                .label(
                    egui::RichText::new("⠿")
                        .size(17.0)
                        .color(chrome::EDGE_STRONG),
                )
                .on_hover_text("drag to rearrange");
        });
        if chrome::icon(ui, "×")
            .on_hover_text("delete filter")
            .clicked()
        {
            actions.push(Action::Delete(filter.name.clone()));
        }
        if chrome::icon(ui, "⧉").on_hover_text("clone").clicked() {
            actions.push(Action::Clone(filter.name.clone()));
        }
        let selected = active == Some(&filter.name);
        let label = if selected {
            format!("● {}", filter.name)
        } else {
            filter.name.to_string()
        };
        // Tooltip only when the name is actually truncated — otherwise it just
        // echoes a name you can already read.
        let font = egui::TextStyle::Button.resolve(ui.style());
        let natural = ui
            .painter()
            .layout_no_wrap(label.clone(), font, egui::Color32::PLACEHOLDER)
            .size()
            .x;
        let truncated = natural > ui.available_width();
        let response = ui.selectable_label(selected, label);
        let response = if truncated {
            response.on_hover_text(filter.name.as_str())
        } else {
            response
        };
        if chrome::hover_started(ui, &response) {
            actions.push(Action::Pulse(response.rect));
        }
        if response.clicked() {
            actions.push(Action::Load(filter.clone()));
        }
    });
    let rect = row.response.rect;
    let after = ui
        .ctx()
        .pointer_latest_pos()
        .is_some_and(|pos| pos.y > rect.center().y);
    if let Some(payload) = row.response.dnd_hover_payload::<FilterName>()
        && *payload != filter.name
    {
        let y = if after { rect.bottom() } else { rect.top() };
        let _line = ui
            .painter()
            .hline(rect.x_range(), y, egui::Stroke::new(1.0, chrome::HOT));
    }
    if let Some(payload) = row.response.dnd_release_payload::<FilterName>()
        && *payload != filter.name
    {
        actions.push(Action::Moor {
            name: (*payload).clone(),
            berth: Berth::Beside {
                anchor: filter.name.clone(),
                after,
            },
        });
    }
}

fn shelf_rows(
    ui: &mut egui::Ui,
    slot: usize,
    shelf: &Shelf,
    active: Option<&FilterName>,
    shelf_edit: &mut Option<ShelfEdit>,
    actions: &mut Vec<Action>,
) {
    let id = ui.make_persistent_id(("filter-shelf", slot));
    let header = ui.horizontal(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let glyph = if shelf.open { "▾" } else { "▸" };
        if chrome::glyph(ui, glyph, false).clicked() {
            actions.push(Action::ToggleShelf(slot));
        }
        if chrome::icon(ui, "✎")
            .on_hover_text("rename folder")
            .clicked()
        {
            actions.push(Action::BeginShelfRename(slot));
        }
        if chrome::icon(ui, "×")
            .on_hover_text("delete folder (filters spill out)")
            .clicked()
        {
            actions.push(Action::ScuttleShelf(slot));
        }
        match shelf_edit {
            Some(edit) if edit.shelf == slot => {
                let before = edit.name.clone();
                let entry = ui.text_edit_singleline(&mut edit.name);
                if let Some(wake) = chrome::text_wake(ui, &entry, &before, &edit.name) {
                    actions.push(Action::TypeWake(wake));
                }
                if edit.focus {
                    entry.request_focus();
                    edit.focus = false;
                }
                let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                if entry.lost_focus() || (entry.has_focus() && enter) {
                    actions.push(Action::CommitShelfRename);
                }
            }
            _ => {
                let _name = ui.label(chrome::section_title(format!(
                    "{} ({})",
                    shelf.name,
                    shelf.filters.len()
                )));
            }
        }
    });
    let rect = header.response.rect;
    if header.response.dnd_hover_payload::<FilterName>().is_some() {
        let _glow = ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, chrome::HOT),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(payload) = header.response.dnd_release_payload::<FilterName>() {
        actions.push(Action::Moor {
            name: (*payload).clone(),
            berth: Berth::Shelf(slot),
        });
    }
    if shelf.open {
        let _body = ui.indent(id.with("body"), |ui| {
            if shelf.filters.is_empty() {
                let _empty = ui.label(chrome::muted("empty"));
            }
            for filter in &shelf.filters {
                filter_row(ui, active, filter, actions);
            }
        });
    }
}

/// While a drag is live, a strip at the bottom of the library that moors the
/// payload back into the root list.
fn root_basin(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    if !egui::DragAndDrop::has_any_payload(ui.ctx()) {
        return;
    }
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
    let hot = response.dnd_hover_payload::<FilterName>().is_some();
    let stroke = egui::Stroke::new(1.0, if hot { chrome::HOT } else { chrome::EDGE });
    let _line = ui.painter().hline(rect.x_range(), rect.center().y, stroke);
    if let Some(payload) = response.dnd_release_payload::<FilterName>() {
        actions.push(Action::Moor {
            name: (*payload).clone(),
            berth: Berth::Root,
        });
    }
}
