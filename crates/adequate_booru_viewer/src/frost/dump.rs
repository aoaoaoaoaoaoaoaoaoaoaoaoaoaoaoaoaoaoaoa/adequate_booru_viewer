use super::*;
use anyhow::{Context as _, Result};
use std::{
    io::Write as _,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8] = b"ABV_WATER_DUMP\0";

impl Frost {
    pub fn dump(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
        surge: &Surge<'_>,
        screen: [u32; 2],
        pixels_per_point: f32,
    ) -> Result<()> {
        let rig = self.rig.as_ref().context("missing frost rig")?;
        let mut blob = Vec::with_capacity(1024);
        blob.extend_from_slice(MAGIC);
        section(
            &mut blob,
            "meta.txt",
            self.meta(rig, surge, screen, pixels_per_point).as_bytes(),
        );
        section(&mut blob, "mask.bin", &mask_bytes(surge));
        for slot in 0..2 {
            let bytes = read_texture(
                device,
                queue,
                &rig.water.textures[slot],
                rig.water.size,
                SIM_BYTES,
            )
            .with_context(|| format!("read water texture {slot}"))?;
            section(&mut blob, &format!("water{slot}.rg32f"), &bytes);
        }
        if let Some(stride) = format_stride(self.format) {
            let size = wgpu::Extent3d {
                width: screen[0].max(1),
                height: screen[1].max(1),
                depth_or_array_layers: 1,
            };
            let scene = read_texture(device, queue, &rig.scene.texture, size, stride)
                .context("read egui scene texture")?;
            section(&mut blob, "scene.raw", &scene);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let mut file =
            std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
        file.write_all(&blob)
            .with_context(|| format!("write {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", path.display()))
    }

    fn meta(
        &self,
        rig: &Rig,
        surge: &Surge<'_>,
        screen: [u32; 2],
        pixels_per_point: f32,
    ) -> String {
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |age| age.as_millis());
        format!(
            "\
version = 1
unix_ms = {unix_ms}
surface_width = {}
surface_height = {}
pixels_per_point = {pixels_per_point}
target_format = {:?}
sim_format = {:?}
sim_bytes = {SIM_BYTES}
field_scale = {FIELD_SCALE}
sim_steps = {SIM_STEPS}
sim_dt = {SIM_DT}
water_width = {}
water_height = {}
water_phase = {}
dry = {}
wake = {}
tide = {}
scroll_tilt = {}
tension_count = {}
lift_count = {}
splash_count = {}
touch_count = {}
raft = {}
brine = {:#?}
",
            screen[0],
            screen[1],
            self.format,
            SIM_FORMAT,
            rig.water.size.width,
            rig.water.size.height,
            rig.water.phase,
            surge.dry,
            surge.wake,
            surge.tide,
            surge.scroll_tilt,
            surge.tensions.len(),
            surge.lifts.len(),
            surge.splashes.len(),
            surge.touches.len(),
            surge.raft.is_some(),
            surge.brine,
        )
    }
}

fn section(blob: &mut Vec<u8>, name: &str, bytes: &[u8]) {
    blob.extend_from_slice(&(name.len() as u32).to_le_bytes());
    blob.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    blob.extend_from_slice(name.as_bytes());
    blob.extend_from_slice(bytes);
}

fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: wgpu::Extent3d,
    bytes_per_pixel: u32,
) -> Result<Vec<u8>> {
    let row = size.width * bytes_per_pixel;
    let pitch =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("water-dump-readback"),
        size: u64::from(pitch) * u64::from(size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("water-dump-readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    let ticket = queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _sent = tx.send(result);
    });
    let _drained = device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(ticket),
            timeout: Some(Duration::from_secs(10)),
        })
        .context("wait water dump readback")?;
    rx.recv_timeout(Duration::from_secs(10))
        .context("receive water dump map result")?
        .context("map water dump readback")?;
    let view = slice.get_mapped_range();
    let mut bytes = Vec::with_capacity((row * size.height) as usize);
    for y in 0..size.height {
        let start = (y * pitch) as usize;
        bytes.extend_from_slice(&view[start..start + row as usize]);
    }
    drop(view);
    buffer.unmap();
    Ok(bytes)
}

fn format_stride(format: wgpu::TextureFormat) -> Option<u32> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm
        | wgpu::TextureFormat::Rgba8UnormSrgb
        | wgpu::TextureFormat::Bgra8Unorm
        | wgpu::TextureFormat::Bgra8UnormSrgb => Some(4),
        wgpu::TextureFormat::Rgba16Float => Some(8),
        wgpu::TextureFormat::Rgba32Float => Some(16),
        _ => None,
    }
}
