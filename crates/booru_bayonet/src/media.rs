use anyhow::{Context as _, Result, bail};
use image::ImageReader;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
use ureq::Agent;

use crate::model::PostId;

#[derive(Clone, Debug)]
pub struct RgbaBlade {
    pub id: PostId,
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
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
        let config = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("booru_bayonet/0.1 anonymous-readonly")
            .build();
        Ok(Self {
            root,
            agent: config.into(),
        })
    }

    pub fn blade(&self, id: PostId, url: &str) -> Result<RgbaBlade> {
        let path = self.path_for(id, url);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let bytes = self.fetch(url)?;
                std::fs::write(&path, &bytes)
                    .with_context(|| format!("write {}", path.display()))?;
                bytes
            }
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        };
        decode(id, &bytes)
    }

    fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .with_context(|| format!("GET media {url}"))?;
        response.body_mut().read_to_vec().context("read media body")
    }

    fn path_for(&self, id: PostId, url: &str) -> PathBuf {
        self.root.join(format!("{}.{}", id.0, extension(url)))
    }
}

fn decode(id: PostId, bytes: &[u8]) -> Result<RgbaBlade> {
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

fn extension(url: &str) -> &str {
    let path = url.split('?').next().unwrap_or(url);
    let Some(ext) = Path::new(path).extension().and_then(|ext| ext.to_str()) else {
        return "img";
    };
    if ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext
    } else {
        "img"
    }
}

pub fn required_url(url: Option<&str>) -> Result<&str> {
    let Some(url) = url else {
        bail!("post has no media URL");
    };
    Ok(url)
}
