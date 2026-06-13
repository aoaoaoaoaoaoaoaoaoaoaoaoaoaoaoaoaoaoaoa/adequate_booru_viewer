use super::*;
use std::{
    fmt,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

const PERIOD: Duration = Duration::from_millis(850);
const HEIGHT_LIMIT: f32 = 96.0;
const VELOCITY_LIMIT: f32 = 2880.0;
const HEIGHT_RAIL: f32 = 47.0;
const VELOCITY_RAIL: f32 = 1410.0;
const SATURATION_MIN_CELLS: u32 = 512;
const SATURATION_DENOMINATOR: u32 = 200;

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
        let saturation_limit =
            SATURATION_MIN_CELLS.max(self.size.width * self.size.height / SATURATION_DENOMINATOR);
        let mut saturated = 0_u32;
        for y in 0..self.size.height {
            let row = (y * self.pitch) as usize;
            for x in 0..self.size.width {
                let px = row + (x * SIM_BYTES) as usize;
                debug_assert!(px + SIM_BYTES as usize <= bytes.len());
                for channel in 0..2 {
                    let at = px + channel * 4;
                    let bits = u32::from_le_bytes([
                        bytes[at],
                        bytes[at + 1],
                        bytes[at + 2],
                        bytes[at + 3],
                    ]);
                    let raw = f32::from_bits(bits);
                    let value = raw.abs();
                    if !raw.is_finite() {
                        return Some(Fault {
                            kind: FaultKind::Nonfinite,
                            x,
                            y,
                            channel,
                            bits,
                            value,
                        });
                    }
                    let limit = if channel == 0 {
                        HEIGHT_LIMIT
                    } else {
                        VELOCITY_LIMIT
                    };
                    if value > limit {
                        return Some(Fault {
                            kind: FaultKind::Oversize,
                            x,
                            y,
                            channel,
                            bits,
                            value,
                        });
                    }
                    let rail = if channel == 0 {
                        HEIGHT_RAIL
                    } else {
                        VELOCITY_RAIL
                    };
                    if value >= rail {
                        saturated += 1;
                        if saturated > saturation_limit {
                            return Some(Fault {
                                kind: FaultKind::Saturated { saturated },
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
    pub(super) fn clear(&self, queue: &wgpu::Queue) {
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
    kind: FaultKind,
    x: u32,
    y: u32,
    channel: usize,
    bits: u32,
    value: f32,
}

enum FaultKind {
    Nonfinite,
    Oversize,
    Saturated { saturated: u32 },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            FaultKind::Nonfinite => "nonfinite",
            FaultKind::Oversize => "oversize",
            FaultKind::Saturated { saturated } => {
                return write!(
                    f,
                    "saturation flood after {saturated} railed samples near ({}, {}) channel {} bits=0x{:08x} value={}",
                    self.x, self.y, self.channel, self.bits, self.value
                );
            }
        };
        write!(
            f,
            "{kind} at ({}, {}) channel {} bits=0x{:08x} value={}",
            self.x, self.y, self.channel, self.bits, self.value
        )
    }
}
