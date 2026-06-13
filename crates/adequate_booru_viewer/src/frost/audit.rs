use super::*;
use anyhow::{Context as _, Result, bail};
use std::{sync::mpsc, time::Duration};

const SURFACE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const W: u32 = 640;
const H: u32 = 360;

#[test]
fn poisoned_water_cells_are_extinguished() -> Result<()> {
    pollster::block_on(async {
        let Some(mut bench) = Bench::make().await? else {
            return Ok(());
        };
        bench.poison(37, 29)?;
        bench.step(&quiet(0.0))?;
        let field = bench.field()?;
        field.assert_clean()?;
        field.assert_quiet(37, 29, 0.25)
    })
}

#[test]
fn water_guard_resets_poisoned_field() -> Result<()> {
    pollster::block_on(async {
        let Some(mut bench) = Bench::make().await? else {
            return Ok(());
        };
        bench.poison(37, 29)?;
        if bench.field()?.assert_clean().is_ok() {
            bail!("water poison write did not land");
        }
        if !bench.guard()? {
            bail!("water guard did not report a reset");
        }
        let field = bench.field()?;
        field.assert_clean()?;
        field.assert_quiet(37, 29, 0.25)
    })
}

#[test]
fn aggressive_water_script_never_writes_nonfinite_state() -> Result<()> {
    pollster::block_on(async {
        let Some(mut bench) = Bench::make().await? else {
            return Ok(());
        };
        for frame in 0..180 {
            let script = Script::storm(frame);
            bench.step(&script.surge(frame as f32 / 60.0))?;
            if frame % 15 == 0 {
                bench.field()?.assert_clean()?;
            }
        }
        bench.field()?.assert_clean()
    })
}

#[test]
fn water_allocates_half_resolution_field() -> Result<()> {
    pollster::block_on(async {
        let Some(bench) = Bench::make().await? else {
            return Ok(());
        };
        bench.assert_size(W.div_ceil(2), H.div_ceil(2))
    })
}

struct Bench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    frost: Frost,
}

impl Bench {
    async fn make() -> Result<Option<Self>> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(desc);
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
        {
            Ok(adapter) => adapter,
            Err(err) => {
                eprintln!("water audit skipped: no wgpu adapter: {err}");
                return Ok(None);
            }
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("water-audit"),
                ..Default::default()
            })
            .await
            .context("request water audit device")?;
        let mut frost = Frost::new(&device, SURFACE);
        frost.resize(&device, &queue, W, H);
        Ok(Some(Self {
            device,
            queue,
            frost,
        }))
    }

    fn step(&mut self, surge: &Surge<'_>) -> Result<()> {
        let rig = self.frost.rig.as_mut().context("missing frost rig")?;
        self.queue
            .write_buffer(&self.frost.mask, 0, &mask_bytes(surge));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("water-audit-step"),
            });
        for _ in 0..SIM_STEPS {
            run_compute(
                &mut encoder,
                &self.frost.pipes.sim,
                &rig.water.sim_bind[rig.water.phase],
                rig.water.size,
            );
            rig.water.phase ^= 1;
        }
        let ticket = self.queue.submit([encoder.finish()]);
        let _drained = self
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(ticket),
                timeout: Some(Duration::from_secs(10)),
            })
            .context("wait water audit step")?;
        Ok(())
    }

    fn poison(&mut self, x: u32, y: u32) -> Result<()> {
        let rig = self.frost.rig.as_ref().context("missing frost rig")?;
        let bits = [f32::INFINITY.to_le_bytes(), f32::INFINITY.to_le_bytes()].concat();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &rig.water.textures[rig.water.phase],
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &bits,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: None,
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    fn field(&self) -> Result<Field> {
        let rig = self.frost.rig.as_ref().context("missing frost rig")?;
        Field::read(&self.device, &self.queue, &rig.water)
    }

    fn guard(&mut self) -> Result<bool> {
        let rig = self.frost.rig.as_ref().context("missing frost rig")?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("water-audit-guard"),
            });
        self.frost
            .sentinel
            .encode(&self.device, &mut encoder, &rig.water);
        let submitted = self.queue.submit([encoder.finish()]);
        if self.frost.after_submit(&self.device, &self.queue, true) {
            return Ok(true);
        }
        for _ in 0..20 {
            let _drained = self
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submitted.clone()),
                    timeout: Some(Duration::from_millis(50)),
                })
                .context("wait water guard readback")?;
            if self.frost.after_submit(&self.device, &self.queue, true) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn assert_size(&self, width: u32, height: u32) -> Result<()> {
        let rig = self.frost.rig.as_ref().context("missing frost rig")?;
        let size = rig.water.size;
        if (size.width, size.height) != (width, height) {
            bail!(
                "water is {}×{}, expected {width}×{height}",
                size.width,
                size.height
            );
        }
        Ok(())
    }
}

struct Field {
    bytes: Vec<u8>,
    width: u32,
}

impl Field {
    fn read(device: &wgpu::Device, queue: &wgpu::Queue, water: &Water) -> Result<Self> {
        let row = water.size.width * SIM_BYTES;
        let pitch =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water-audit-readback"),
            size: u64::from(pitch) * u64::from(water.size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("water-audit-readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &water.textures[water.phase],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(water.size.height),
                },
            },
            water.size,
        );
        let ticket = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _sent = tx.send(result);
        });
        let _drained = device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(ticket),
                timeout: Some(Duration::from_secs(10)),
            })
            .context("wait water audit readback")?;
        rx.recv_timeout(Duration::from_secs(10))
            .context("receive water audit map result")?
            .context("map water audit readback")?;
        let view = slice.get_mapped_range();
        let mut bytes = Vec::with_capacity((row * water.size.height) as usize);
        for y in 0..water.size.height {
            let start = (y * pitch) as usize;
            bytes.extend_from_slice(&view[start..start + row as usize]);
        }
        drop(view);
        buffer.unmap();
        Ok(Self {
            bytes,
            width: water.size.width,
        })
    }

    fn assert_clean(&self) -> Result<()> {
        for (cell, chunk) in self.bytes.chunks_exact(SIM_BYTES as usize).enumerate() {
            for channel in 0..2 {
                let at = channel * 4;
                let value =
                    f32::from_le_bytes([chunk[at], chunk[at + 1], chunk[at + 2], chunk[at + 3]]);
                if !value.is_finite() {
                    bail!(
                        "nonfinite f32 in water field at ({}, {}), channel {}, bits=0x{:08x}",
                        cell as u32 % self.width,
                        cell as u32 / self.width,
                        channel,
                        value.to_bits(),
                    );
                }
            }
        }
        Ok(())
    }

    fn assert_quiet(&self, x: u32, y: u32, limit: f32) -> Result<()> {
        let at = ((y * self.width + x) * SIM_BYTES) as usize;
        for channel in 0..2 {
            let at = at + channel * 4;
            let value = f32::from_le_bytes([
                self.bytes[at],
                self.bytes[at + 1],
                self.bytes[at + 2],
                self.bytes[at + 3],
            ]);
            if value.abs() > limit {
                bail!(
                    "poisoned cell survived as {} in channel {channel}, limit {limit}",
                    value.abs(),
                );
            }
        }
        Ok(())
    }
}

fn quiet(tide: f32) -> Surge<'static> {
    Surge {
        dry: false,
        veil: None,
        tensions: &[],
        lifts: &[],
        water: water_rect(),
        scroll_tilt: 0.0,
        splashes: &[],
        raft: None,
        viewer: far_rect(),
        touches: &[],
        wake: true,
        tide,
        brine: Brine::default(),
        guard: true,
    }
}

struct Script {
    tensions: Vec<Tension>,
    lifts: Vec<Lift>,
    splashes: Vec<Splash>,
    raft: Option<Raft>,
}

impl Script {
    fn storm(frame: usize) -> Self {
        let mut tensions = Vec::with_capacity(QUIVER_SLOTS);
        let mut lifts = Vec::with_capacity(LIFT_SLOTS);
        let mut splashes = Vec::with_capacity(SPLASH_SLOTS);
        for i in 0..QUIVER_SLOTS {
            let x = 120.0 + ((frame * 37 + i * 83) % 460) as f32;
            let y = 24.0 + ((frame * 29 + i * 71) % 300) as f32;
            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(44.0, 20.0));
            tensions.push(Tension {
                id: i as u64,
                rect,
                pointer: rect.center(),
                grip: 0.35 + i as f32 * 0.18,
                omega: if i % 2 == 0 { 0.0 } else { 0.3 * TAU },
            });
        }
        for i in 0..LIFT_SLOTS {
            let x = 104.0 + ((frame * 53 + i * 97) % 430) as f32;
            let y = 18.0 + ((frame * 47 + i * 67) % 270) as f32;
            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(92.0, 118.0));
            lifts.push(if i == 3 {
                Lift::shallow(rect, 0.78)
            } else {
                Lift::surface(rect, 0.35 + i as f32 * 0.19)
            });
        }
        for i in 0..SPLASH_SLOTS {
            let x = 96.0 + ((frame * 23 + i * 31) % 500) as f32;
            let y = ((frame * 19 + i * 43) % 330) as f32;
            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(72.0, 96.0));
            splashes.push(Splash {
                rect,
                age: (i as f32 % 11.0) * 0.023,
                amp: 10.0 + i as f32 % 7.0,
                shape: SplashShape::Ring,
            });
        }
        let raft = frame.is_multiple_of(3).then(|| {
            let rect = egui::Rect::from_min_size(egui::pos2(240.0, 96.0), egui::vec2(180.0, 96.0));
            Raft {
                rect,
                corners: [
                    1.0 + (frame as f32 * 0.07).sin() * 4.0,
                    2.0 + (frame as f32 * 0.11).cos() * 3.5,
                    3.0 + (frame as f32 * 0.13).sin() * 3.0,
                    1.5 + (frame as f32 * 0.17).cos() * 4.5,
                ],
            }
        });
        Self {
            tensions,
            lifts,
            splashes,
            raft,
        }
    }

    fn surge(&self, tide: f32) -> Surge<'_> {
        Surge {
            dry: false,
            veil: None,
            tensions: &self.tensions,
            lifts: &self.lifts,
            water: water_rect(),
            scroll_tilt: ((tide * 2.3).sin() * 14.0).clamp(-18.0, 18.0),
            splashes: &self.splashes,
            raft: self.raft,
            viewer: far_rect(),
            touches: &[],
            wake: true,
            tide,
            brine: Brine::default(),
            guard: true,
        }
    }
}

fn water_rect() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(96.0, 0.0), egui::pos2(W as f32, H as f32))
}

fn far_rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(-4e6, -4e6), egui::Vec2::ZERO)
}
