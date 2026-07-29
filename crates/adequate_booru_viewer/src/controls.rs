//! ABV-owned controls that are not mechanisms in the Poolrooms foundry.

use crate::{chrome, water};

/// A textual command or mutually-exclusive choice. Text has no honest rigid
/// foundry geometry, so it remains an application plate rather than
/// masquerading as a [`chrome::Monoglyph`].
pub fn plate(ui: &mut egui::Ui, text: impl Into<String>, selected: bool) -> egui::Response {
    plate_enabled(ui, true, text, selected)
}

pub fn plate_enabled(
    ui: &mut egui::Ui,
    enabled: bool,
    text: impl Into<String>,
    selected: bool,
) -> egui::Response {
    let text = egui::RichText::new(text.into())
        .size(13.0)
        .strong()
        .color(if selected { chrome::HOT } else { chrome::TEXT });
    let button = egui::Button::new(text).min_size(egui::vec2(24.0, 20.0));
    let button = if selected {
        button
            .fill(chrome::RAISED)
            .stroke(egui::Stroke::new(1.4_f32, chrome::HOT))
    } else {
        button
    };
    let response = ui.add_enabled(enabled, button);
    chrome::tension(ui, &response);
    response
}

/// Sink one single-glyph foundry plunger into the active water surface.
pub fn plunger(
    ui: &mut egui::Ui,
    water: &mut water::Surface,
    glyph: char,
) -> chrome::MonoglyphResponse {
    let response = chrome::Monoglyph::new(glyph).show(ui);
    water.monoglyph(&response);
    response
}
