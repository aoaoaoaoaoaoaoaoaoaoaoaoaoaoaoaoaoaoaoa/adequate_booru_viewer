//! The tide bench: a wall of sliders over every water constant, for live
//! calibration. Debug surface only — toggled with F12, never advertised.

use std::f32::consts::TAU;

type Knob<'a> = (&'a str, &'a mut f32, std::ops::RangeInclusive<f32>);

impl super::Bayonet {
    pub(super) fn bench(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(egui::Key::F12)) {
            self.bench_open = !self.bench_open;
        }
        if !self.bench_open {
            return;
        }
        let mut open = self.bench_open;
        let _window = egui::Window::new("tide bench")
            .open(&mut open)
            .default_width(340.0)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.spacing_mut().slider_width = 170.0;
                let brine = &mut self.brine;
                let surf = &mut self.surf;
                // The tremor reads better in wavelength/frequency than in
                // wavenumber/angular rate; convert at the bench's edge.
                let mut lambda = TAU / brine.tremor_k;
                let mut hertz = brine.tremor_omega / TAU;
                let sections: [(&str, Vec<Knob<'_>>); 7] = [
                    (
                        "tension",
                        vec![
                            ("pull reach px", &mut brine.reach, 5.0..=120.0),
                            ("meniscus px", &mut brine.meniscus_px, 0.0..=8.0),
                            ("water gain", &mut brine.refract_px, 0.0..=4.0),
                            ("chroma ∂n", &mut brine.ior_spread, 0.0..=1.2),
                        ],
                    ),
                    (
                        "quiver",
                        vec![
                            ("bulge px", &mut brine.quiver_bulge, 0.0..=10.0),
                            ("bulge pulse", &mut brine.quiver_pulse, 0.0..=1.0),
                            ("tremor λ px", &mut lambda, 6.0..=80.0),
                            ("tremor Hz", &mut hertz, 0.0..=3.0),
                            ("tremor amp px", &mut brine.tremor_amp, 0.0..=3.0),
                            ("tremor fade px", &mut brine.tremor_fade, 5.0..=300.0),
                            ("tremor reach px", &mut brine.tremor_reach, 20.0..=500.0),
                        ],
                    ),
                    (
                        "lift",
                        vec![
                            (
                                "bulge px",
                                &mut brine.bulge_px,
                                0.0..=crate::frost::BULGE_CEIL,
                            ),
                            ("brighten", &mut brine.lift_bright, 0.0..=0.4),
                            ("rise τ s", &mut surf.tau_rise, 0.02..=1.5),
                            ("sink τ s", &mut surf.tau_fall, 0.02..=1.5),
                        ],
                    ),
                    (
                        "waves",
                        vec![
                            ("crest speed px/s", &mut brine.wave_v, 40.0..=900.0),
                            ("swell σ px", &mut brine.wave_sigma, 3.0..=60.0),
                            ("damping s", &mut brine.wave_damp, 0.2..=6.0),
                            ("spreading px", &mut brine.wave_spread, 30.0..=1000.0),
                        ],
                    ),
                    (
                        "splashes",
                        vec![
                            ("enter amp px", &mut surf.enter_amp, 0.0..=12.0),
                            ("exit amp px", &mut surf.exit_amp, 0.0..=12.0),
                            ("click amp px", &mut surf.click_amp, 0.0..=12.0),
                            ("viewer ring life s", &mut surf.viewer_life, 0.5..=10.0),
                        ],
                    ),
                    (
                        "scroll surge",
                        vec![
                            ("surge every px", &mut surf.surge_quantum, 8.0..=240.0),
                            ("surge amp px", &mut surf.surge_amp, 0.0..=16.0),
                            ("surge τ s", &mut surf.surge_tau, 0.02..=0.8),
                        ],
                    ),
                    (
                        "shore",
                        vec![
                            ("panel transmit", &mut brine.t_panel, 0.0..=1.0),
                            ("panel reflect", &mut brine.r_panel, 0.0..=1.0),
                            ("wall reflect", &mut brine.r_wall, 0.0..=1.0),
                            ("feather px", &mut brine.shore_feather, 1.0..=60.0),
                        ],
                    ),
                ];
                for (title, knobs) in sections {
                    let _title = ui.label(egui::RichText::new(title).color(crate::chrome::MUTED));
                    for (label, value, range) in knobs {
                        let _slider = ui.add(egui::Slider::new(value, range).text(label));
                    }
                    ui.add_space(6.0);
                }
                brine.tremor_k = TAU / lambda.max(1.0);
                brine.tremor_omega = hertz * TAU;
                if ui.button("becalm (reset all)").clicked() {
                    self.brine = crate::frost::Brine::default();
                    self.surf = super::Surf::default();
                }
            });
        self.bench_open = open;
    }
}
