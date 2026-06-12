use super::*;
use std::{
    fmt,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

const PERIOD: Duration = Duration::from_millis(850);
const HEIGHT_LIMIT: f32 = 96.0;
const VELOCITY_LIMIT: f32 = 2880.0;

#[derive(Default)]
pub(super) struct Sentinel {
    next: Option<Instant>,
    probe: Option<Probe>,
}

impl Sentinel {
    pub(super) fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        water: &Water,
    ) {
        let now = Instant::now();
        if self.probe.is_some() || self.next.is_some_and(|next| next > now) {
            return;
        }
        self.next = now.checked_add(PERIOD);
        let readback = Readback::new(device, water.size);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &water.textures[water.phase],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(readback.pitch),
                    rows_per_image: Some(water.size.height),
                },
            },
            water.size,
        );
        self.probe = Some(Probe::Copied {
            readback,
            submitted: false,
        });
    }

    pub(super) fn after_submit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        water: &Water,
    ) -> bool {
        self.arm_mapping();
        let _polled = device.poll(wgpu::PollType::Poll);
        let Some(Probe::Mapping(mapping)) = &self.probe else {
            return false;
        };
        match mapping.rx.try_recv() {
            Ok(Ok(())) => {
                let mapping = match self.probe.take() {
                    Some(Probe::Mapping(mapping)) => mapping,
                    _ => unreachable!("probe state changed while resolving map"),
                };
                if let Some(fault) = mapping.fault() {
                    eprintln!("water guard reset poisoned field: {fault}");
                    water.clear(queue);
                    true
                } else {
                    false
                }
            }
            Ok(Err(err)) => {
                eprintln!("water guard readback failed; resetting field: {err}");
                self.probe = None;
                water.clear(queue);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                eprintln!("water guard readback channel died; resetting field");
                self.probe = None;
                water.clear(queue);
                true
            }
        }
    }

    pub(super) fn disarm(&mut self) {
        self.probe = None;
    }

    fn arm_mapping(&mut self) {
        let Some(probe) = self.probe.take() else {
            return;
        };
        let Probe::Copied {
            readback,
            submitted,
        } = probe
        else {
            self.probe = Some(probe);
            return;
        };
        if !submitted {
            self.probe = Some(Probe::Copied {
                readback,
                submitted: true,
            });
            return;
        }
        let slice = readback.buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _sent = tx.send(result);
        });
        self.probe = Some(Probe::Mapping(Mapping { readback, rx }));
    }
}

enum Probe {
    Copied { readback: Readback, submitted: bool },
    Mapping(Mapping),
}

struct Mapping {
    readback: Readback,
    rx: Receiver<Result<(), wgpu::BufferAsyncError>>,
}

impl Mapping {
    fn fault(self) -> Option<Fault> {
        let view = self.readback.buffer.slice(..).get_mapped_range();
        let fault = self.readback.fault(&view);
        drop(view);
        self.readback.buffer.unmap();
        fault
    }
}

struct Readback {
    buffer: wgpu::Buffer,
    size: wgpu::Extent3d,
    pitch: u32,
}

impl Readback {
    fn new(device: &wgpu::Device, size: wgpu::Extent3d) -> Self {
        let row = size.width * SIM_BYTES;
        let pitch =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water-guard-readback"),
            size: u64::from(pitch) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            size,
            pitch,
        }
    }

    fn fault(&self, bytes: &[u8]) -> Option<Fault> {
        for y in 0..self.size.height {
            let row = (y * self.pitch) as usize;
            for x in 0..self.size.width {
                let px = row + (x * SIM_BYTES) as usize;
                debug_assert!(px + SIM_BYTES as usize <= bytes.len());
                for channel in 0..4 {
                    let at = px + channel * 2;
                    let bits = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
                    if bits & 0x7c00 == 0x7c00 {
                        return Some(Fault {
                            x,
                            y,
                            channel,
                            bits,
                            value: half_abs(bits),
                        });
                    }
                    if channel < 2 {
                        let value = half_abs(bits);
                        let limit = if channel == 0 {
                            HEIGHT_LIMIT
                        } else {
                            VELOCITY_LIMIT
                        };
                        if value > limit {
                            return Some(Fault {
                                x,
                                y,
                                channel,
                                bits,
                                value,
                            });
                        }
                    }
                }
            }
        }
        None
    }
}

impl Water {
    fn clear(&self, queue: &wgpu::Queue) {
        let zeros = vec![0_u8; (self.size.width * self.size.height * SIM_BYTES) as usize];
        for texture in &self.textures {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &zeros,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.size.width * SIM_BYTES),
                    rows_per_image: Some(self.size.height),
                },
                self.size,
            );
        }
    }
}

struct Fault {
    x: u32,
    y: u32,
    channel: usize,
    bits: u16,
    value: f32,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({}, {}) channel {} bits=0x{:04x} value={}",
            self.x, self.y, self.channel, self.bits, self.value
        )
    }
}

fn half_abs(bits: u16) -> f32 {
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x03ff;
    match exp {
        0 => f32::from(frac) * 2.0_f32.powi(-24),
        31 if frac == 0 => f32::INFINITY,
        31 => f32::NAN,
        _ => (1.0 + f32::from(frac) / 1024.0) * 2.0_f32.powi(i32::from(exp) - 15),
    }
}
