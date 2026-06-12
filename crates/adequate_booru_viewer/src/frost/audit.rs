use super::*;
use anyhow::{Context as _, Result, bail};
use std::{sync::mpsc, time::Duration};

const SURFACE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const W: u32 = 640;
const H: u32 = 360;
const HALF_INF: u16 = 0x7c00;

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
        let pipes = &self.frost.pipes[rig.definition.slot()];
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("water-audit-step"),
            });
        for _ in 0..rig.definition.spec().steps {
            run_compute(
                &mut encoder,
                &pipes.sim,
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
        let bits = [
            HALF_INF.to_le_bytes(),
            HALF_INF.to_le_bytes(),
            0_u16.to_le_bytes(),
            0_u16.to_le_bytes(),
        ]
        .concat();
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
        for (slot, chunk) in self.bytes.chunks_exact(2).enumerate() {
            let bits = u16::from_le_bytes([chunk[0], chunk[1]]);
            if bits & 0x7c00 == 0x7c00 {
                let px = slot / 4;
                bail!(
                    "nonfinite half in water field at ({}, {}), channel {}, bits=0x{bits:04x}",
                    px as u32 % self.width,
                    px as u32 / self.width,
                    slot % 4,
                );
            }
        }
        Ok(())
    }

    fn assert_quiet(&self, x: u32, y: u32, limit: f32) -> Result<()> {
        let at = ((y * self.width + x) * SIM_BYTES) as usize;
        for channel in 0..2 {
            let bits = u16::from_le_bytes([
                self.bytes[at + channel * 2],
                self.bytes[at + channel * 2 + 1],
            ]);
            let value = f16(bits).abs();
            if value > limit {
                bail!("poisoned cell survived as {value} in channel {channel}, limit {limit}");
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
        viewer: far_rect(),
        touches: &[],
        wake: true,
        tide,
        brine: Brine::default(),
    }
}

struct Script {
    tensions: Vec<Tension>,
    lifts: Vec<Lift>,
    splashes: Vec<Splash>,
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
            });
        }
        Self {
            tensions,
            lifts,
            splashes,
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
            viewer: far_rect(),
            touches: &[],
            wake: true,
            tide,
            brine: Brine::default(),
        }
    }
}

fn water_rect() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(96.0, 0.0), egui::pos2(W as f32, H as f32))
}

fn far_rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(-4e6, -4e6), egui::Vec2::ZERO)
}

fn f16(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x03ff;
    match exp {
        0 => sign * f32::from(frac) * 2.0_f32.powi(-24),
        31 if frac == 0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + f32::from(frac) / 1024.0) * 2.0_f32.powi(i32::from(exp) - 15),
    }
}
