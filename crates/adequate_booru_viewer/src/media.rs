use anyhow::{Context as _, Result, bail};
use image::{ImageBuffer, ImageReader, Rgba, imageops::FilterType};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use ureq::Agent;

use crate::model::{PostId, media_extension};

/// Monotonic claim for temp-file names: concurrent fetchers of the same id
/// must never share a temp path.
static CLAIM: AtomicU64 = AtomicU64::new(0);
const MAX_MEDIA_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct RgbaBlade {
    pub id: PostId,
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

impl RgbaBlade {
    /// Forge a display-sized derivative with a proper reconstruction filter.
    /// GPU bilinear sampling alone aliases when an original is many times
    /// larger than its viewport.
    pub fn downsample(&self, bounds: [u32; 2]) -> Option<Self> {
        let width = u32::try_from(self.size[0]).ok()?;
        let height = u32::try_from(self.size[1]).ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        let scale = (f64::from(bounds[0].max(1)) / f64::from(width))
            .min(f64::from(bounds[1].max(1)) / f64::from(height))
            .min(1.0);
        if scale >= 1.0 {
            return None;
        }
        let target_width = (f64::from(width) * scale).round().max(1.0) as u32;
        let target_height = (f64::from(height) * scale).round().max(1.0) as u32;
        let source = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, self.rgba.as_slice())?;
        let derivative =
            image::imageops::resize(&source, target_width, target_height, FilterType::Lanczos3);
        Some(Self {
            id: self.id,
            size: [target_width as usize, target_height as usize],
            rgba: derivative.into_raw(),
        })
    }
}

#[derive(Clone)]
pub struct MediaCache {
    root: PathBuf,
    agent: Agent,
}

impl MediaCache {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("create media cache {}", root.display()))?;
        // One pooled connection per fetcher (plus one spare), held long
        // enough to survive human browsing pauses without a re-handshake.
        let config = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .max_idle_connections_per_host(4)
            .max_idle_age(Duration::from_secs(90))
            .user_agent("adequate_booru_viewer/0.1 anonymous-readonly")
            .build();
        Ok(Self {
            root,
            agent: config.into(),
        })
    }

    pub fn blade(&self, id: PostId, url: &str) -> Result<RgbaBlade> {
        let bytes = self.bytes(id, url)?;
        decode(id, &bytes)
    }

    pub fn bytes(&self, id: PostId, url: &str) -> Result<Vec<u8>> {
        let path = cache_path(&self.root, id, url);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let bytes = self.fetch(url)?;
                // Temp + rename: a crash or a racing fetcher can never leave
                // a torn file that would poison decoding forever. No fsync —
                // this is a cache; partial loss is fine, partial files not.
                let claim = CLAIM.fetch_add(1, Ordering::Relaxed);
                let tmp = path.with_extension(format!("part{claim:x}"));
                std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
                std::fs::rename(&tmp, &path)
                    .with_context(|| format!("install {}", path.display()))?;
                Ok(bytes)
            }
            Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
        }
    }

    fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .with_context(|| format!("GET media {url}"))?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_MEDIA_BYTES)
            .read_to_vec()
            .context("read media body")
    }
}

pub fn cached(root: &Path, id: PostId, url: &str) -> bool {
    cache_path(root, id, url).is_file()
}

pub fn cache_path(root: &Path, id: PostId, url: &str) -> PathBuf {
    root.join(format!("{}-{:016x}.{}", id.0, fnv1a(url), extension(url)))
}

fn decode(id: PostId, bytes: &[u8]) -> Result<RgbaBlade> {
    // JPEG is the bulk of booru bytes; zune-jpeg decodes it several times
    // faster than the image crate, in pure Rust on every platform. Anything
    // else (or a JPEG zune rejects) falls through to the image crate.
    if bytes.starts_with(&[0xFF, 0xD8])
        && let Some(blade) = decode_jpeg_fast(id, bytes)
    {
        return Ok(blade);
    }
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("guess media format")?
        .decode()
        .context("decode media")?
        .to_rgba8();
    let (w, h) = image.dimensions();
    Ok(RgbaBlade {
        id,
        size: [w as usize, h as usize],
        rgba: image.into_raw(),
    })
}

fn decode_jpeg_fast(id: PostId, bytes: &[u8]) -> Option<RgbaBlade> {
    let options = zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes), options);
    let rgba = decoder.decode().ok()?;
    let (width, height) = decoder.dimensions()?;
    (rgba.len() == width * height * 4).then_some(RgbaBlade {
        id,
        size: [width, height],
        rgba,
    })
}

pub fn extension(url: &str) -> &str {
    media_extension(url).unwrap_or("img")
}

fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn required_url(url: Option<&str>) -> Result<&str> {
    let Some(url) = url else {
        bail!("post has no media URL");
    };
    Ok(url)
}
