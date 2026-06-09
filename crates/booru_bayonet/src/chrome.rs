use eframe::egui::{self, Color32, RichText, Stroke, Vec2};

pub const PAGE: Color32 = Color32::from_rgb(4, 8, 13);
pub const SURFACE: Color32 = Color32::from_rgb(6, 10, 16);
pub const RAISED: Color32 = Color32::from_rgb(15, 36, 48);
pub const CONTROL: Color32 = Color32::from_rgb(4, 12, 18);
pub const EDGE: Color32 = Color32::from_rgb(0, 104, 128);
pub const EDGE_STRONG: Color32 = Color32::from_rgb(68, 152, 176);
pub const TEXT: Color32 = Color32::from_rgb(210, 226, 236);
pub const MUTED: Color32 = Color32::from_rgb(159, 180, 202);
pub const HOT: Color32 = Color32::from_rgb(159, 215, 234);

pub fn install(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PAGE;
    visuals.window_fill = SURFACE;
    visuals.faint_bg_color = CONTROL;
    visuals.extreme_bg_color = Color32::from_rgb(3, 10, 15);
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.inactive.bg_fill = CONTROL;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, EDGE);
    visuals.widgets.hovered.bg_fill = RAISED;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, EDGE_STRONG);
    visuals.widgets.active.bg_fill = Color32::from_rgb(8, 26, 36);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, HOT);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(28, 152, 252, 92);
    visuals.selection.stroke = Stroke::new(1.0, HOT);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = Vec2::splat(6.0);
    style.spacing.button_padding = Vec2::new(7.0, 3.0);
    style.spacing.window_margin = egui::Margin::symmetric(8, 8);
    style.spacing.menu_margin = egui::Margin::symmetric(8, 8);
    style.spacing.indent = 12.0;
    ctx.set_global_style(style);
}

pub fn section(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    title: &'static str,
    default_open: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    let response = egui::CollapsingHeader::new(section_title(title))
        .id_salt(id)
        .default_open(default_open)
        .show(ui, |ui| {
            surface(ui, add);
        });
    let _response = response
        .header_response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
}

pub fn surface(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let _frame = egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, EDGE))
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, add);
}

pub fn section_title(text: &'static str) -> RichText {
    RichText::new(text)
        .size(13.0)
        .strong()
        .color(HOT)
        .text_style(egui::TextStyle::Button)
}

pub fn eyebrow(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(11.0)
        .color(MUTED)
        .text_style(egui::TextStyle::Small)
}

pub fn title(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).size(17.0).strong().color(TEXT)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).size(12.0).color(MUTED)
}
