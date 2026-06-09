use anyhow::{Context as _, Result, anyhow, bail};
use image::{
    ImageReader,
    imageops::{FilterType, crop_imm, resize},
};
use ort::{
    session::Session,
    value::{DynTensor, Tensor},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
use ureq::Agent;

use crate::model::{CLIP_DIM, Embedding};

const REPO: &str = "jinaai/jina-clip-v1";
const REV: &str = "ceb3e44ca4d6eceaa4f3fb58b1c1a5748b3f29b6";
const VISION_MODEL: &str = "onnx/vision_model_quantized.onnx";
const IMAGE_SIZE: u32 = 224;
const IMAGE_PIXELS: usize = IMAGE_SIZE as usize * IMAGE_SIZE as usize;
const MODEL_BYTE_LIMIT: u64 = 600 * 1024 * 1024;
const CLIP_MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const CLIP_STD: [f32; 3] = [0.268_629_54, 0.261_302_6, 0.275_777_1];

pub struct ClipForge {
    root: PathBuf,
    agent: Agent,
    vision: Option<Session>,
}

impl ClipForge {
    pub fn new(root: PathBuf) -> Self {
        let agent = agent();
        Self {
            root,
            agent,
            vision: None,
        }
    }

    pub fn image(&mut self, bytes: &[u8]) -> Result<Embedding> {
        let pixels = pixels(bytes)?;
        let mut inputs = std::collections::HashMap::<String, DynTensor>::new();
        for name in self.vision()?.inputs().iter().map(|input| input.name()) {
            match name {
                "pixel_values" => {
                    let tensor = Tensor::from_array((
                        [1_usize, 3, IMAGE_SIZE as usize, IMAGE_SIZE as usize],
                        pixels.clone(),
                    ))?;
                    let _old = inputs.insert(name.to_owned(), tensor.upcast());
                }
                other => bail!("unsupported Jina CLIP vision input `{other}`"),
            }
        }
        let outputs = self.vision()?.run(inputs).context("run Jina CLIP vision")?;
        extract_embedding(&outputs, "image_embeds")
    }

    fn vision(&mut self) -> Result<&mut Session> {
        if self.vision.is_none() {
            let path = ensure_file(&self.agent, &self.root, VISION_MODEL)?;
            self.vision = Some(session(&path).context("load Jina CLIP vision model")?);
        }
        self.vision.as_mut().context("vision session missing")
    }
}

fn agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_mins(20)))
        .user_agent("booru_bayonet/0.1 anonymous-readonly")
        .build();
    config.into()
}

fn session(path: &Path) -> Result<Session> {
    let builder = Session::builder().map_err(|err| anyhow!("{err}"))?;
    let builder = builder
        .with_intra_threads(2)
        .map_err(|err| anyhow!("{err}"))?;
    let mut builder = builder
        .with_inter_threads(1)
        .map_err(|err| anyhow!("{err}"))?;
    builder
        .commit_from_file(path)
        .map_err(|err| anyhow!("{err}"))
        .with_context(|| format!("open ONNX {}", path.display()))
}

fn ensure_file(agent: &Agent, root: &Path, rel: &str) -> Result<PathBuf> {
    let path = root.join(REPO).join(rel);
    if path.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let url = format!("https://huggingface.co/{REPO}/resolve/{REV}/{rel}");
    let mut response = agent
        .get(&url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MODEL_BYTE_LIMIT)
        .read_to_vec()
        .with_context(|| format!("read {url}"))?;
    if bytes.is_empty() {
        bail!("downloaded empty model file {url}");
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("install {} -> {}", tmp.display(), path.display()))?;
    Ok(path)
}

fn pixels(bytes: &[u8]) -> Result<Vec<f32>> {
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("guess image format")?
        .decode()
        .context("decode image for CLIP")?
        .to_rgb8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        bail!("empty image");
    }
    let shortest = width.min(height);
    let new_width = ((u64::from(width) * u64::from(IMAGE_SIZE)) / u64::from(shortest))
        .max(u64::from(IMAGE_SIZE)) as u32;
    let new_height = ((u64::from(height) * u64::from(IMAGE_SIZE)) / u64::from(shortest))
        .max(u64::from(IMAGE_SIZE)) as u32;
    let resized = resize(&image, new_width, new_height, FilterType::CatmullRom);
    let x = (new_width - IMAGE_SIZE) / 2;
    let y = (new_height - IMAGE_SIZE) / 2;
    let cropped = crop_imm(&resized, x, y, IMAGE_SIZE, IMAGE_SIZE).to_image();
    let mut out = vec![0.0_f32; 3 * IMAGE_PIXELS];
    for (i, pixel) in cropped.pixels().enumerate() {
        for channel in 0..3 {
            out[channel * IMAGE_PIXELS + i] =
                (f32::from(pixel[channel]) / 255.0 - CLIP_MEAN[channel]) / CLIP_STD[channel];
        }
    }
    Ok(out)
}

fn extract_embedding(outputs: &ort::session::SessionOutputs<'_>, name: &str) -> Result<Embedding> {
    let Some(output) = outputs.get(name) else {
        let keys = outputs.keys().collect::<Vec<_>>().join(", ");
        bail!("Jina CLIP produced no `{name}` output; outputs: {keys}");
    };
    let (_, data) = output
        .try_extract_tensor::<f32>()
        .with_context(|| format!("extract `{name}` tensor"))?;
    if data.len() != CLIP_DIM {
        let keys = outputs.keys().collect::<Vec<_>>().join(", ");
        bail!(
            "expected {CLIP_DIM}-wide `{name}` embedding, got {} floats; outputs: {keys}",
            data.len()
        );
    }
    Embedding::forge(data.to_vec())
}
