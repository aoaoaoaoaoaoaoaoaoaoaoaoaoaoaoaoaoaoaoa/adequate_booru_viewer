use eframe::egui;

use crate::{
    chrome,
    config::{FilterName, SavedFilter},
};

#[derive(Clone, Debug)]
pub enum Action {
    New,
    Save,
    Rename,
    Load(SavedFilter),
    Clone(FilterName),
    Delete(FilterName),
}

pub fn active_card(
    ui: &mut egui::Ui,
    name_entry: &mut String,
    active: Option<&FilterName>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let _eyebrow = ui.label(chrome::eyebrow("ACTIVE FILTER"));
    let _title = ui.label(match active {
        Some(name) => chrome::title(name.to_string()),
        None => chrome::title("new unsaved filter"),
    });
    let _mode = ui.label(match active {
        Some(_) => chrome::muted("autosave is armed for query edits"),
        None => chrome::muted("scratch query; save to keep it in the library"),
    });
    ui.add_space(3.0);
    let _save = ui.horizontal(|ui| {
        let entry = ui.add(egui::TextEdit::singleline(name_entry).hint_text("name / rename"));
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
        if let Some(active) = active
            && ui.button("clone").clicked()
        {
            actions.push(Action::Clone(active.clone()));
        }
    });
    actions
}

pub fn library(
    ui: &mut egui::Ui,
    active: Option<&FilterName>,
    filters: &[SavedFilter],
) -> Vec<Action> {
    let mut actions = Vec::new();
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
            if ui.selectable_label(selected, label).clicked() {
                actions.push(Action::Load(filter.clone()));
            }
            if ui.small_button("clone").clicked() {
                actions.push(Action::Clone(filter.name.clone()));
            }
        });
    }
    actions
}
