use super::*;

impl Bayonet {
    pub fn frost_frame(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
        tooltip_rects: &[egui::Rect],
    ) -> crate::frost::Frame {
        self.water.set_wetness(wetness(self.water_mode));
        let veil = self.frost_veil(ctx);
        self.water.frame(ctx, pixels_per_point, tooltip_rects, veil)
    }

    pub(super) fn heave(&mut self, ctx: &egui::Context, offset: f32) {
        self.water.heave(ctx, offset);
    }

    pub(super) fn bump_plunge(&mut self, rect: egui::Rect) {
        self.water.bump(rect);
    }

    pub(super) fn lever_plunge(&mut self, rect: egui::Rect, sign: f32) {
        self.water.lever(rect, sign);
    }

    pub(super) fn tape_plunge(&mut self, rect: egui::Rect, travel: f32) {
        self.water.drag(rect, travel);
    }

    pub(super) fn group_plunge(&mut self, rect: egui::Rect) {
        self.water.select(rect);
    }

    pub(super) fn pool_thwack(&mut self, rect: egui::Rect, energy: f32) {
        self.water.thwack(rect, energy);
    }

    pub(super) fn fold_plunge(&mut self, wake: Option<chrome::FoldWake>) {
        self.water.fold(wake);
    }

    pub(super) fn text_plunge(&mut self, wake: chrome::TextWake) {
        self.water.text(wake);
    }

    pub(super) fn touch_viewer(&mut self, center: egui::Pos2) {
        self.water.touch(center);
    }
}

fn wetness(mode: WaterMode) -> crate::frost::Wetness {
    match mode {
        WaterMode::Dry => crate::frost::Wetness::Dry,
        WaterMode::Wet => crate::frost::Wetness::Wet,
        WaterMode::ReallyWet => crate::frost::Wetness::Deluge,
    }
}
