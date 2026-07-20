use super::*;

impl Bayonet {
    pub fn water_frame(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
        tooltip_rects: &[egui::Rect],
    ) -> crate::water::Frame {
        let veil = self.water_veil(ctx);
        let wetness = wetness(self.water_mode);
        self.water.set_wetness(wetness);
        self.family_water.set_wetness(wetness);
        *self.family_water.chemistry_mut() = *self.water.chemistry();
        *self.family_water.agitation_mut() = *self.water.agitation();
        match self.viewer_surface {
            ViewerSurface::Image => self.water.frame(ctx, pixels_per_point, tooltip_rects, veil),
            ViewerSurface::Family => {
                self.family_water
                    .frame(ctx, pixels_per_point, tooltip_rects, veil)
            }
        }
    }
}

fn wetness(mode: WaterMode) -> crate::water::Wetness {
    match mode {
        WaterMode::Dry => crate::water::Wetness::Dry,
        WaterMode::Wet => crate::water::Wetness::Wet,
        WaterMode::ReallyWet => crate::water::Wetness::Deluge,
    }
}
