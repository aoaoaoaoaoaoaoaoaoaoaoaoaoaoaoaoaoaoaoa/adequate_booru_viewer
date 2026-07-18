//! F12 water-physics bench. This is not product UI; it is a live calibration
//! console over high-leverage constants in the near-physics model.

use std::f32::consts::TAU;

impl super::Bayonet {
    pub(super) fn bench(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(egui::Key::F12)) {
            self.bench_open = !self.bench_open;
        }
        if !self.bench_open {
            return;
        }
        let mut open = self.bench_open;
        let mut reset = false;
        let _window = egui::Window::new("water physics bench")
            .open(&mut open)
            .default_width(360.0)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.spacing_mut().slider_width = 190.0;
                let (brine, surf) = self.water.laboratory_mut();
                let mut wavelength = TAU / brine.tremor_k;
                let mut hertz = brine.tremor_omega / TAU;

                section(ui, "OPTICS", |ui| {
                    knob(ui, "refraction strength", &mut brine.refract_px, 0.0..=4.0);
                    knob(ui, "chromatic split", &mut brine.ior_spread, 0.0..=1.2);
                });
                section(ui, "BUTTON PLATES", |ui| {
                    knob(ui, "meniscus pull px", &mut brine.meniscus_px, 0.0..=8.0);
                    knob(ui, "meniscus radius px", &mut brine.reach, 5.0..=120.0);
                    knob(ui, "plate lift px", &mut brine.quiver_bulge, 0.0..=10.0);
                    knob(ui, "vibration amp", &mut brine.tremor_amp, 0.0..=4.0);
                    knob(ui, "wavelength px", &mut wavelength, 6.0..=80.0);
                    knob(ui, "frequency Hz", &mut hertz, 0.0..=3.0);
                    knob(
                        ui,
                        "vibration decay px",
                        &mut brine.tremor_fade,
                        5.0..=300.0,
                    );
                    knob(ui, "ring-down s", &mut surf.quiver_release, 0.03..=2.0);
                });
                section(ui, "IMAGE PLATES", |ui| {
                    knob(
                        ui,
                        "lift footprint px",
                        &mut brine.bulge_px,
                        0.0..=crate::frost::BULGE_CEIL,
                    );
                    knob(ui, "surfaced light", &mut brine.lift_bright, 0.0..=0.4);
                    knob(ui, "rise time s", &mut surf.lift_rise, 0.02..=1.5);
                    knob(ui, "sink time s", &mut surf.lift_fall, 0.02..=1.5);
                    knob(ui, "hover impulse", &mut surf.enter_impulse, 0.0..=12.0);
                    knob(ui, "release impulse", &mut surf.exit_impulse, 0.0..=12.0);
                    knob(ui, "open impulse", &mut surf.click_impulse, 0.0..=12.0);
                    knob(ui, "swap thump", &mut surf.thwack_impulse, 0.0..=2.0);
                });
                section(ui, "WATER FIELD", |ui| {
                    knob(ui, "wave speed px/s", &mut brine.wave_v, 40.0..=900.0);
                    knob(ui, "source width px", &mut brine.wave_sigma, 3.0..=60.0);
                    knob(ui, "bulk damping s", &mut brine.wave_damp, 0.2..=6.0);
                    knob(ui, "impulse gain", &mut brine.source_gain, 0.0..=120.0);
                    knob(ui, "tray force gain", &mut brine.tilt_gain, 0.0..=360.0);
                    knob(
                        ui,
                        "height retention",
                        &mut brine.height_retention,
                        0.995..=1.0,
                    );
                });
                section(ui, "SCROLL SLOSH", |ui| {
                    knob(
                        ui,
                        "scroll force per px/s",
                        &mut surf.scroll_coupling,
                        0.0..=0.08,
                    );
                    knob(ui, "tray memory s", &mut surf.scroll_memory, 0.02..=0.8);
                });
                section(ui, "FIELD GUARD", |ui| {
                    let _guard = ui.checkbox(&mut surf.poison_sweep, "poison sweep + reset");
                });
                section(ui, "BOUNDARIES", |ui| {
                    knob(ui, "shelf reflection", &mut brine.r_panel, 0.0..=1.0);
                    knob(ui, "panel shimmer", &mut brine.t_panel, 0.0..=1.0);
                    knob(
                        ui,
                        "boundary softness px",
                        &mut brine.shore_feather,
                        1.0..=60.0,
                    );
                });
                section(ui, "VIEWER + TEXT", |ui| {
                    knob(ui, "typed glyph impulse", &mut surf.text_impulse, 0.0..=4.0);
                    knob(ui, "viewer tap impulse", &mut surf.pond_impulse, 0.0..=8.0);
                    knob(ui, "viewer ring life s", &mut surf.pond_life, 0.5..=10.0);
                    knob(
                        ui,
                        "viewer spreading px",
                        &mut brine.wave_spread,
                        30.0..=1000.0,
                    );
                    knob(ui, "viewer wall reflection", &mut brine.r_wall, 0.0..=1.0);
                });

                brine.tremor_k = TAU / wavelength.max(1.0);
                brine.tremor_omega = hertz * TAU;
                ui.add_space(4.0);
                if ui.button("BECALM: RESET PHYSICS").clicked() {
                    reset = true;
                }
            });
        if reset {
            self.water.reset_laboratory();
        }
        self.bench_open = open;
    }
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    let _title = ui.label(
        egui::RichText::new(title)
            .strong()
            .color(crate::chrome::HOT),
    );
    body(ui);
    ui.add_space(8.0);
}

fn knob(
    ui: &mut egui::Ui,
    label: &'static str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) {
    let _slider = ui.add(egui::Slider::new(value, range).text(label));
}
