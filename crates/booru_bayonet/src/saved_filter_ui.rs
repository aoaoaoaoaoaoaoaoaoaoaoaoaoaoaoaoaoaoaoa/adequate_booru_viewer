use eframe::egui::{self, RichText};

use crate::config::{FilterName, SavedFilter};

#[derive(Clone, Debug)]
pub enum Action {
    New,
    Save,
    Rename,
    Load(SavedFilter),
    Clone(FilterName),
    Delete(FilterName),
}

pub fn render(
    ui: &mut egui::Ui,
    name_entry: &mut String,
    active: Option<&FilterName>,
    filters: &[SavedFilter],
) -> Vec<Action> {
    let mut actions = Vec::new();
    let _heading = ui.heading("saved");
    let _active = ui.label(match active {
        Some(name) => RichText::new(format!("editing: {name}")).strong(),
        None => RichText::new("editing: new unsaved").italics(),
    });
    let _save = ui.horizontal(|ui| {
        let entry =
            ui.add(egui::TextEdit::singleline(name_entry).hint_text("filter name / rename"));
        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if ui.button("new").clicked() {
            actions.push(Action::New);
        }
        if ui.button("save").clicked() || (entry.has_focus() && enter) {
            actions.push(Action::Save);
        }
        if ui
            .add_enabled(active.is_some(), egui::Button::new("rename"))
            .clicked()
        {
            actions.push(Action::Rename);
        }
    });
    if filters.is_empty() {
        let _empty = ui.label("none");
    }
    for filter in filters {
        let selected = active == Some(&filter.name);
        let label = if selected {
            format!("● {}", filter.name)
        } else {
            filter.name.to_string()
        };
        let _row = ui.horizontal_wrapped(|ui| {
            if ui.small_button("×").clicked() {
                actions.push(Action::Delete(filter.name.clone()));
            }
            if ui.small_button("clone").clicked() {
                actions.push(Action::Clone(filter.name.clone()));
            }
            if ui.selectable_label(selected, label).clicked() {
                actions.push(Action::Load(filter.clone()));
            }
        });
    }
    actions
}
