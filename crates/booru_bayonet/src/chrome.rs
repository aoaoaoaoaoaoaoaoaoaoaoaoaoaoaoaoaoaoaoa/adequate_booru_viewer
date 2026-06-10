use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

pub const INSPECTOR_WIDTH: f32 = 380.0;
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
    let id = ui.make_persistent_id(id);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    let _frame = egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, EDGE))
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let header = egui::Frame::new()
                .fill(RAISED)
                .stroke(Stroke::new(1.0, EDGE))
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    let glyph = if state.is_open() { "▾" } else { "▸" };
                    let response = ui
                        .horizontal(|ui| {
                            let _glyph = ui.label(RichText::new(glyph).color(HOT).strong());
                            let _title = ui.label(section_title(title));
                        })
                        .response
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        state.toggle(ui);
                    }
                });
            if state.is_open() {
                let _body = egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(9, 7))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        add(ui);
                    });
            }
            state.store(ui.ctx());
            header.response
        });
}

pub fn section_title(text: &'static str) -> RichText {
    RichText::new(text)
        .size(13.0)
        .strong()
        .color(HOT)
        .text_style(egui::TextStyle::Button)
}

pub fn glyph_button(text: impl Into<String>, selected: bool) -> egui::Button<'static> {
    let text = RichText::new(text.into())
        .size(13.0)
        .strong()
        .color(if selected { HOT } else { TEXT });
    egui::Button::new(text)
        .fill(if selected { RAISED } else { CONTROL })
        .stroke(Stroke::new(
            if selected { 1.4 } else { 1.0 },
            if selected { HOT } else { EDGE },
        ))
        .min_size(Vec2::new(24.0, 20.0))
}

pub fn icon_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(14.0).color(HOT))
        .fill(CONTROL)
        .stroke(Stroke::new(1.0, EDGE))
        .min_size(Vec2::splat(22.0))
}

pub fn rail_f32(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> egui::Response {
    let start = *range.start();
    let end = *range.end();
    let old = *value;
    let (rect, mut response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 22.0),
        Sense::click_and_drag(),
    );
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        *value = start + t * (end - start);
    }
    if (*value - old).abs() > f32::EPSILON {
        response.mark_changed();
    }
    paint_rail(ui, rect, ((*value - start) / (end - start)).clamp(0.0, 1.0));
    response
}

pub fn rail_u16(
    ui: &mut egui::Ui,
    value: &mut u16,
    range: std::ops::RangeInclusive<u16>,
) -> egui::Response {
    rail_u16_sized(ui, value, range, ui.available_width())
}

pub fn rail_u16_sized(
    ui: &mut egui::Ui,
    value: &mut u16,
    range: std::ops::RangeInclusive<u16>,
    width: f32,
) -> egui::Response {
    let start = *range.start();
    let end = *range.end();
    let old = *value;
    let (rect, mut response) = ui.allocate_exact_size(
        egui::vec2(width.min(ui.available_width()), 22.0),
        Sense::click_and_drag(),
    );
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let span = f32::from(end.saturating_sub(start));
        *value = (f32::from(start) + t * span).round() as u16;
    }
    *value = (*value).clamp(start, end);
    if *value != old {
        response.mark_changed();
    }
    let span = f32::from(end.saturating_sub(start)).max(1.0);
    paint_rail(ui, rect, f32::from((*value).saturating_sub(start)) / span);
    response
}

fn paint_rail(ui: &mut egui::Ui, rect: egui::Rect, t: f32) {
    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.center().y - 3.0),
        egui::pos2(rect.right(), rect.center().y + 3.0),
    );
    let x = egui::lerp(track.left()..=track.right(), t);
    let fill = egui::Rect::from_min_max(track.min, egui::pos2(x, track.max.y));
    let thumb = egui::Rect::from_center_size(egui::pos2(x, track.center().y), Vec2::new(8.0, 18.0));
    let _track = ui.painter().rect_filled(track, 0.0, CONTROL);
    let _track_stroke =
        ui.painter()
            .rect_stroke(track, 0.0, Stroke::new(1.0, EDGE), egui::StrokeKind::Inside);
    let _fill = ui.painter().rect_filled(fill, 0.0, EDGE_STRONG);
    let _thumb = ui.painter().rect_filled(thumb, 0.0, HOT);
    let _thumb_stroke = ui.painter().rect_stroke(
        thumb,
        0.0,
        Stroke::new(1.0, Color32::from_rgb(2, 7, 10)),
        egui::StrokeKind::Inside,
    );
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
