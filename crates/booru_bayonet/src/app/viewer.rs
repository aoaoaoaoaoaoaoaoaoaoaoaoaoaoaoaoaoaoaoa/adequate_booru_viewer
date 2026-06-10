use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ViewerBar {
    pub(super) copy: bool,
    pub(super) save: bool,
}

pub(super) fn viewer_title_bar(
    ui: &mut egui::Ui,
    title: &str,
    can_save: bool,
    close: &mut bool,
) -> ViewerBar {
    let mut bar = ViewerBar::default();
    let _bar = egui::Frame::new()
        .fill(chrome::RAISED)
        .stroke(egui::Stroke::new(1.0, chrome::EDGE))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let _row = ui.horizontal(|ui| {
                let _title = ui.label(
                    egui::RichText::new(title)
                        .size(13.0)
                        .strong()
                        .color(chrome::TEXT),
                );
                let _actions =
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(chrome::icon_button("×"))
                            .on_hover_text("close")
                            .clicked()
                        {
                            *close = true;
                        }
                        if ui
                            .add_enabled(can_save, chrome::glyph_button("save", false))
                            .clicked()
                        {
                            bar.save = true;
                        }
                        if ui.add(chrome::glyph_button("copy", false)).clicked() {
                            bar.copy = true;
                        }
                    });
            });
        });
    bar
}
