use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, RichText, Sense, Stroke, Vec2};

const CMU_TYPEWRITER: &[u8] = include_bytes!("../assets/fonts/cmu-typewriter/cmuntt.ttf");
const NOTO_MATH: &[u8] = include_bytes!("../assets/fonts/noto/NotoSansMath-Regular.ttf");
const NOTO_SYMBOLS: &[u8] = include_bytes!("../assets/fonts/noto/NotoSansSymbols2-Regular.ttf");

const FACE_TEXT: &str = "cmu-typewriter-text";
const FACE_MATH: &str = "noto-sans-math";
const FACE_SYMBOLS: &str = "noto-sans-symbols-2";

// Ink and lamplight: warm charcoal paper, bone text, umber edges, lamplight
// amber for the accent, typewriter-ribbon red for repulsion — tuned to sit
// with the CMU Typewriter face instead of fighting it.
pub const INSPECTOR_WIDTH: f32 = 285.0;
pub const PAGE: Color32 = Color32::from_rgb(12, 11, 9);
pub const SURFACE: Color32 = Color32::from_rgb(17, 15, 12);
pub const RAISED: Color32 = Color32::from_rgb(36, 30, 22);
pub const CONTROL: Color32 = Color32::from_rgb(13, 12, 10);
pub const EDGE: Color32 = Color32::from_rgb(78, 66, 48);
pub const EDGE_STRONG: Color32 = Color32::from_rgb(142, 125, 100);
pub const TEXT: Color32 = Color32::from_rgb(226, 217, 198);
pub const MUTED: Color32 = Color32::from_rgb(158, 147, 128);
pub const HOT: Color32 = Color32::from_rgb(235, 197, 151);

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PAGE;
    visuals.window_fill = SURFACE;
    visuals.faint_bg_color = CONTROL;
    visuals.extreme_bg_color = Color32::from_rgb(9, 8, 7);
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.inactive.bg_fill = CONTROL;
    visuals.widgets.inactive.weak_bg_fill = CONTROL;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, EDGE);
    visuals.widgets.hovered.bg_fill = RAISED;
    visuals.widgets.hovered.weak_bg_fill = RAISED;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, EDGE_STRONG);
    visuals.widgets.active.bg_fill = Color32::from_rgb(44, 36, 25);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(44, 36, 25);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, HOT);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(226, 172, 106, 60);
    visuals.selection.stroke = Stroke::new(1.0, HOT);
    visuals.hyperlink_color = HOT;
    visuals.window_stroke = Stroke::new(1.0, EDGE_STRONG);
    // Near-flat: containers get 2px corners, widgets 1px.
    visuals.window_corner_radius = egui::CornerRadius::same(2);
    visuals.menu_corner_radius = egui::CornerRadius::same(2);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(1);
    }
    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = Vec2::splat(6.0);
    style.spacing.button_padding = Vec2::new(7.0, 3.0);
    style.spacing.window_margin = egui::Margin::symmetric(8, 8);
    style.spacing.menu_margin = egui::Margin::symmetric(8, 8);
    style.spacing.indent = 12.0;
    ctx.set_global_style(style);
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (face, bytes) in [
        (FACE_TEXT, CMU_TYPEWRITER),
        (FACE_MATH, NOTO_MATH),
        (FACE_SYMBOLS, NOTO_SYMBOLS),
    ] {
        let _old = fonts
            .font_data
            .insert(face.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        prepend_faces(&mut fonts, family, &[FACE_TEXT, FACE_MATH, FACE_SYMBOLS]);
    }
    ctx.set_fonts(fonts);
}

fn prepend_faces(fonts: &mut FontDefinitions, family: FontFamily, faces: &[&str]) {
    let stack = fonts.families.entry(family).or_default();
    for face in faces.iter().rev() {
        stack.retain(|name| name != face);
        stack.insert(0, (*face).to_owned());
    }
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
        .corner_radius(2)
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
                    let _row = ui.horizontal(|ui| {
                        let _glyph = ui.label(RichText::new(glyph).color(HOT).strong());
                        let _title = ui.label(section_title(title.to_ascii_uppercase()));
                    });
                });
            let header_click = ui
                .interact(header.response.rect, id.with("header"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if header_click.clicked() {
                state.toggle(ui);
            }
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

pub fn section_title(text: impl Into<String>) -> RichText {
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
    // Explicit fill/stroke would override the hover and press visuals; only
    // the selected state pins its look.
    let button = egui::Button::new(text).min_size(Vec2::new(24.0, 20.0));
    if selected {
        button.fill(RAISED).stroke(Stroke::new(1.4, HOT))
    } else {
        button
    }
}

/// How fast tension grips and releases. Short, with a fast-rising ease so
/// the refraction reads as "on" the instant the pointer lands rather than
/// creeping up linearly.
const TENSION_TIME: f32 = 0.09;

/// Puts a hovered widget "in tension": records a seed the frost composite
/// turns into a refraction toward the pointer (blue bent hardest). The seed
/// is per-frame temp data; the boiler consumes it after the UI pass.
pub fn tension(ui: &egui::Ui, response: &egui::Response) {
    let hovered = response.hovered();
    let grip = ui.ctx().animate_bool_with_time_and_easing(
        response.id.with("tension"),
        hovered,
        TENSION_TIME,
        egui::emath::easing::cubic_out,
    );
    if grip <= 0.0 {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_default::<Vec<crate::frost::Tension>>(egui::Id::new("tension-field"))
            .push(crate::frost::Tension {
                rect: response.rect.shrink(1.0),
                pointer,
                grip,
            });
    });
}

pub fn glyph(ui: &mut egui::Ui, text: impl Into<String>, selected: bool) -> egui::Response {
    let response = ui.add(glyph_button(text, selected));
    tension(ui, &response);
    response
}

pub fn glyph_enabled(
    ui: &mut egui::Ui,
    enabled: bool,
    text: impl Into<String>,
    selected: bool,
) -> egui::Response {
    let response = ui.add_enabled(enabled, glyph_button(text, selected));
    tension(ui, &response);
    response
}

pub fn icon(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    let response = ui.add(icon_button(text));
    tension(ui, &response);
    response
}

pub fn small(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    let response = ui.small_button(RichText::new(text.into()));
    tension(ui, &response);
    response
}

pub fn text_wake(
    ui: &egui::Ui,
    response: &egui::Response,
    before: &str,
    after: &str,
) -> Option<(egui::Rect, f32)> {
    if !response.changed() {
        return None;
    }
    let old = before.chars().count();
    let new = after.chars().count();
    if new <= old {
        return None;
    }
    let added = new.saturating_sub(old).min(8);
    let weight = after
        .chars()
        .rev()
        .take(added)
        .map(glyph_weight)
        .sum::<f32>()
        .max(0.25);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap(after.to_owned(), font, Color32::PLACEHOLDER);
    let rect = response.rect;
    let x = (rect.left() + 6.0 + text.size().x).clamp(rect.left() + 6.0, rect.right() - 6.0);
    let h = (rect.height() * 0.62).clamp(8.0, 18.0);
    let w = (4.0 + weight * 3.5).clamp(5.0, rect.width() * 0.35);
    Some((
        egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(w, h)),
        weight,
    ))
}

fn glyph_weight(ch: char) -> f32 {
    match ch {
        ' ' | '\t' => 0.35,
        'i' | 'l' | 'I' | '!' | '|' | '\'' | '`' | '.' | ',' | ':' | ';' => 0.55,
        'm' | 'w' | 'M' | 'W' | '@' | '#' | '%' | '&' => 1.35,
        'A'..='Z' | '0'..='9' => 1.1,
        _ if ch.is_ascii_punctuation() => 0.75,
        _ => 1.0,
    }
}

pub fn icon_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(14.0).color(HOT)).min_size(Vec2::splat(22.0))
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

pub fn note(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(muted(text)).wrap())
}
