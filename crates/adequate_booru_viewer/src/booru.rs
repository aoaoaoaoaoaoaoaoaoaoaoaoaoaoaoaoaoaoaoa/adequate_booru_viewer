use anyhow::{Context as _, Result};
use base64::prelude::{BASE64_STANDARD, Engine as _};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use ureq::Agent;

use crate::{
    configuration::DanbooruAccountConfig,
    model::{
        Harvest, Kin, PostId, PostRecord, Query, Rating, Sort, Tag, TagHint, TagKind,
        narrow_post_id,
    },
};

const POST_LIMIT: &str = "200";
const DANBOORU_BUSY_COOLOFF: Duration = Duration::from_secs(30);

pub fn post_url(id: PostId) -> String {
    format!("https://danbooru.donmai.us/posts/{id}")
}

pub trait Booru {
    fn posts(&self, query: &Query, sort: Sort, page: u32) -> Result<Vec<Harvest>>;

    /// Optional provider capability. A backend without a tag-reference corpus
    /// returns `None`; callers must not synthesize network affordances for it.
    fn tag_definitions(&self) -> Option<&dyn TagDefinitionSource> {
        None
    }
}

pub trait TagDefinitionSource {
    fn tag_definition(
        &self,
        tag: &Tag,
    ) -> std::result::Result<Option<TagDefinition>, TagDefinitionFault>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagDefinition {
    pub title: String,
    pub blocks: Vec<DefinitionBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DefinitionBlock {
    Heading(String),
    Paragraph(String),
    Bullet(String),
}

#[derive(Debug)]
pub struct TagDefinitionFault {
    pub message: String,
    pub cooloff: Option<Duration>,
}

#[derive(Clone)]
pub struct Danbooru {
    agent: Agent,
}

impl Danbooru {
    pub fn new() -> Self {
        Self {
            agent: danbooru_agent(&format!(
                "adequate_booru_viewer/{} anonymous-readonly",
                env!("CARGO_PKG_VERSION")
            )),
        }
    }

    pub fn crawl_page(&self, before: Option<PostId>) -> Result<Vec<Harvest>> {
        self.fetch("order:id_desc", before.map(|id| format!("b{}", id.0)))
    }

    pub fn single(&self, id: PostId) -> Result<Vec<Harvest>> {
        self.fetch(&format!("id:{id}"), None)
    }

    pub fn posts_by_ids(&self, ids: &[PostId]) -> Result<Vec<Harvest>> {
        let ids = ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.fetch(&format!("id:{ids}"), None)
    }

    pub fn kin_page(&self, before: Option<PostId>) -> Result<Vec<Kin>> {
        self.fetch_kin("order:id_desc", before.map(|id| format!("b{}", id.0)))
    }

    pub fn kin_single(&self, id: PostId) -> Result<Option<Kin>> {
        self.fetch_kin(&format!("id:{id}"), None)
            .map(|mut posts| posts.pop())
    }

    pub fn kin_children(&self, parent: PostId) -> Result<Vec<Kin>> {
        self.fetch_kin(&format!("parent:{parent}"), None)
    }

    fn fetch_tag_definition(
        &self,
        tag: &Tag,
    ) -> std::result::Result<Option<TagDefinition>, TagDefinitionFault> {
        let mut response = self
            .agent
            .get("https://danbooru.donmai.us/wiki_pages.json")
            .query("search[title]", tag.as_str())
            .query("limit", "1")
            .query("only", "title,body,is_deleted")
            .config()
            .http_status_as_error(false)
            .build()
            .call()
            .map_err(|err| TagDefinitionFault {
                message: format!("fetch Danbooru definition for {tag}: {err}"),
                cooloff: None,
            })?;
        let status = response.status();
        if !status.is_success() {
            let detail = response
                .body_mut()
                .read_json::<DanbooruApiFault>()
                .ok()
                .and_then(DanbooruApiFault::detail);
            let transient = matches!(status.as_u16(), 429 | 502..=504);
            let message = if transient {
                format!("Danbooru definitions temporarily unavailable ({status})")
            } else {
                format!("Danbooru rejected definition lookup ({status})")
            };
            return Err(TagDefinitionFault {
                message: detail.map_or(message.clone(), |detail| format!("{message}: {detail}")),
                cooloff: transient.then_some(DANBOORU_BUSY_COOLOFF),
            });
        }
        let pages = response
            .body_mut()
            .read_json::<Vec<DanbooruWikiPage>>()
            .map_err(|err| TagDefinitionFault {
                message: format!("decode Danbooru definition for {tag}: {err}"),
                cooloff: None,
            })?;
        Ok(pages
            .into_iter()
            .find(|page| !page.is_deleted && page.title.eq_ignore_ascii_case(tag.as_str()))
            .and_then(TagDefinition::from_danbooru))
    }

    fn fetch_kin(&self, tags: &str, page: Option<String>) -> Result<Vec<Kin>> {
        let mut request = self
            .agent
            .get("https://danbooru.donmai.us/posts.json")
            .query("limit", POST_LIMIT)
            .query("tags", tags)
            .query("only", "id,parent_id,has_children");
        if let Some(page) = page {
            request = request.query("page", page);
        }
        let mut response = request.call().context("GET Danbooru kin page")?;
        response
            .body_mut()
            .read_json::<Vec<DanbooruKin>>()
            .context("decode Danbooru kin JSON")?
            .into_iter()
            .map(Kin::try_from)
            .collect()
    }

    fn fetch(&self, tags: &str, page: Option<String>) -> Result<Vec<Harvest>> {
        let mut request = self
            .agent
            .get("https://danbooru.donmai.us/posts.json")
            .query("limit", POST_LIMIT)
            .query("tags", tags);
        if let Some(page) = page {
            request = request.query("page", page);
        }
        let mut response = request.call().context("GET Danbooru posts")?;
        let wire = response
            .body_mut()
            .read_json::<Vec<DanbooruPost>>()
            .context("decode Danbooru posts JSON")?;
        wire.into_iter().map(Harvest::try_from).collect()
    }
}

fn danbooru_agent(user_agent: &str) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .max_idle_age(Duration::from_secs(90))
        .user_agent(user_agent)
        .build()
        .into()
}

pub struct DanbooruAccount {
    agent: Agent,
    authorization: String,
}

#[derive(Debug)]
pub struct TagEditFault {
    pub message: String,
    /// Authoritative state recovered after an ambiguous mutation failure.
    pub harvest: Option<Box<Harvest>>,
}

impl DanbooruAccount {
    pub fn open(config: &DanbooruAccountConfig, config_dir: &Path) -> Result<Self> {
        let login = config.login.trim();
        anyhow::ensure!(!login.is_empty(), "Danbooru login is empty");
        anyhow::ensure!(
            !login.contains([':', '\r', '\n']),
            "Danbooru login contains an invalid character"
        );
        let api_key_path = config.resolve_api_key(config_dir);
        let metadata = std::fs::metadata(&api_key_path)
            .with_context(|| format!("read API key metadata at {}", api_key_path.display()))?;
        anyhow::ensure!(
            metadata.is_file(),
            "API key path is not a regular file: {}",
            api_key_path.display()
        );
        let raw = std::fs::read_to_string(&api_key_path)
            .with_context(|| format!("read API key at {}", api_key_path.display()))?;
        let api_key = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(&raw);
        anyhow::ensure!(!api_key.is_empty(), "Danbooru API key is empty");
        anyhow::ensure!(
            !api_key.contains(['\r', '\n']),
            "Danbooru API key file must contain exactly one line"
        );
        let authorization = format!(
            "Basic {}",
            BASE64_STANDARD.encode(format!("{login}:{api_key}"))
        );
        Ok(Self {
            agent: danbooru_agent(&format!(
                "adequate_booru_viewer/{} configured-account",
                env!("CARGO_PKG_VERSION")
            )),
            authorization,
        })
    }

    pub fn add_post_tags(
        &self,
        id: PostId,
        additions: &[Tag],
    ) -> std::result::Result<Harvest, TagEditFault> {
        let current = self.fetch_public_post(id).map_err(|err| TagEditFault {
            message: format!("fetch current post {id}: {err:#}"),
            harvest: None,
        })?;
        let old_tag_string = current.tag_string.clone();
        let mut tags = old_tag_string
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut known = tags.iter().cloned().collect::<BTreeSet<_>>();
        let mut changed = false;
        for tag in additions {
            if known.insert(tag.as_str().to_owned()) {
                tags.push(tag.as_str().to_owned());
                changed = true;
            }
        }
        if !changed {
            return Harvest::try_from(current).map_err(|err| TagEditFault {
                message: format!("decode current post {id}: {err:#}"),
                harvest: None,
            });
        }
        let tag_string = tags.join(" ");
        let endpoint = format!("https://danbooru.donmai.us/posts/{id}.json");
        let mutation = self
            .agent
            .patch(&endpoint)
            .header("Authorization", &self.authorization)
            .config()
            .http_status_as_error(false)
            .build()
            .send_form([
                ("post[tag_string]", tag_string.as_str()),
                ("post[old_tag_string]", old_tag_string.as_str()),
            ]);
        let (acknowledged, mutation_harvest, mutation_fault) = match mutation {
            Ok(mut response) if response.status().is_success() => {
                match response
                    .body_mut()
                    .read_json::<DanbooruPost>()
                    .context("decode edited Danbooru post")
                    .and_then(Harvest::try_from)
                {
                    Ok(harvest) => (true, Some(harvest), None),
                    Err(err) => (
                        true,
                        None,
                        Some(format!("decode edited post {id}: {err:#}")),
                    ),
                }
            }
            Ok(mut response) => {
                let status = response.status();
                let detail = response
                    .body_mut()
                    .read_json::<DanbooruApiFault>()
                    .ok()
                    .and_then(DanbooruApiFault::edit_detail);
                let failure = detail.map_or_else(
                    || format!("Danbooru rejected edit ({status})"),
                    |detail| format!("Danbooru rejected edit ({status}): {detail}"),
                );
                (false, None, Some(failure))
            }
            Err(err) => (false, None, Some(format!("edit post {id}: {err}"))),
        };
        match self.fetch_public_post(id).and_then(Harvest::try_from) {
            Ok(harvest) if harvest_contains(&harvest, additions) => Ok(harvest),
            Ok(harvest) => Err(TagEditFault {
                message: if acknowledged {
                    let omission = format!(
                        "Danbooru acknowledged the edit, but the refreshed post omitted {}",
                        tag_names(additions)
                    );
                    match mutation_fault {
                        Some(detail) => format!("{omission}; {detail}"),
                        None => omission,
                    }
                } else {
                    mutation_fault.unwrap_or_else(|| "Danbooru edit failed".to_owned())
                },
                harvest: Some(Box::new(harvest)),
            }),
            Err(reconcile) => {
                if let Some(harvest) = mutation_harvest.as_ref()
                    && harvest_contains(harvest, additions)
                {
                    return Ok(harvest.clone());
                }
                let reconciliation = format!("refresh edited post {id}: {reconcile:#}");
                let message = match mutation_fault {
                    Some(failure) => format!("{failure}; {reconciliation}"),
                    None => {
                        if acknowledged {
                            reconciliation
                        } else {
                            format!("edit outcome unknown; {reconciliation}")
                        }
                    }
                };
                Err(TagEditFault {
                    message,
                    harvest: mutation_harvest.map(Box::new),
                })
            }
        }
    }

    fn fetch_public_post(&self, id: PostId) -> Result<DanbooruPost> {
        let mut response = self
            .agent
            .get(format!("https://danbooru.donmai.us/posts/{id}.json"))
            .call()
            .with_context(|| format!("GET Danbooru post {id}"))?;
        response
            .body_mut()
            .read_json::<DanbooruPost>()
            .with_context(|| format!("decode Danbooru post {id}"))
    }
}

#[derive(Debug, Deserialize)]
struct DanbooruApiFault {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

impl DanbooruApiFault {
    fn edit_detail(self) -> Option<String> {
        if self.error.as_deref() == Some("User::PrivilegeError") {
            Some("API key does not permit posts:update from this IP".to_owned())
        } else {
            self.detail()
        }
    }

    fn detail(self) -> Option<String> {
        let message = self.message.filter(|detail| !detail.is_empty());
        let error = self.error.filter(|detail| !detail.is_empty());
        match (message, error) {
            (Some(message), Some(error)) if message != error => {
                Some(format!("{message} ({error})"))
            }
            (Some(message), _) => Some(message),
            (None, error) => error,
        }
    }
}

impl Booru for Danbooru {
    fn posts(&self, query: &Query, sort: Sort, page: u32) -> Result<Vec<Harvest>> {
        self.fetch(&query.remote_seed(sort), Some(page.to_string()))
    }

    fn tag_definitions(&self) -> Option<&dyn TagDefinitionSource> {
        Some(self)
    }
}

impl TagDefinitionSource for Danbooru {
    fn tag_definition(
        &self,
        tag: &Tag,
    ) -> std::result::Result<Option<TagDefinition>, TagDefinitionFault> {
        self.fetch_tag_definition(tag)
    }
}

fn harvest_contains(harvest: &Harvest, additions: &[Tag]) -> bool {
    additions
        .iter()
        .all(|addition| harvest.post.tags.contains(addition))
}

fn tag_names(tags: &[Tag]) -> String {
    tags.iter().map(Tag::as_str).collect::<Vec<_>>().join(", ")
}

#[derive(Debug, Deserialize)]
struct DanbooruWikiPage {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    is_deleted: bool,
}

#[derive(Debug, Deserialize)]
struct DanbooruPost {
    id: u64,
    #[serde(default)]
    parent_id: Option<u64>,
    #[serde(default)]
    has_children: bool,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    score: i32,
    #[serde(default)]
    fav_count: u32,
    #[serde(default)]
    image_width: u32,
    #[serde(default)]
    image_height: u32,
    #[serde(default)]
    rating: String,
    #[serde(default)]
    file_ext: String,
    #[serde(default)]
    tag_string: String,
    #[serde(default)]
    tag_string_general: String,
    #[serde(default)]
    tag_string_artist: String,
    #[serde(default)]
    tag_string_copyright: String,
    #[serde(default)]
    tag_string_character: String,
    #[serde(default)]
    tag_string_meta: String,
    #[serde(default)]
    preview_file_url: Option<String>,
    #[serde(default)]
    large_file_url: Option<String>,
    #[serde(default)]
    file_url: Option<String>,
    #[serde(default)]
    media_asset: Option<DanbooruMediaAsset>,
}

#[derive(Debug, Deserialize)]
struct DanbooruKin {
    id: u64,
    #[serde(default)]
    parent_id: Option<u64>,
    #[serde(default)]
    has_children: bool,
}

#[derive(Debug, Deserialize)]
struct DanbooruMediaAsset {
    #[serde(default)]
    variants: Vec<DanbooruVariant>,
}

#[derive(Debug, Deserialize)]
struct DanbooruVariant {
    #[serde(rename = "type")]
    kind: String,
    url: String,
}

impl TagDefinition {
    fn from_danbooru(page: DanbooruWikiPage) -> Option<Self> {
        let blocks = dtext_blocks(&page.body);
        (!blocks.is_empty()).then_some(Self {
            title: page.title,
            blocks,
        })
    }
}

fn dtext_blocks(body: &str) -> Vec<DefinitionBlock> {
    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut omit_section = false;
    for raw in body.lines() {
        let line = raw.trim();
        if let Some(heading) = dtext_heading(line) {
            flush_paragraph(&mut blocks, &mut paragraph);
            omit_section = omitted_heading(heading);
            if !omit_section {
                blocks.push(DefinitionBlock::Heading(dtext_inline(heading)));
            }
            continue;
        }
        if omit_section {
            continue;
        }
        if line.is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
            continue;
        }
        if let Some(item) = line.strip_prefix("* ") {
            flush_paragraph(&mut blocks, &mut paragraph);
            if !example_reference(item) {
                let item = dtext_inline(item);
                if !item.is_empty() {
                    blocks.push(DefinitionBlock::Bullet(item));
                }
            }
            continue;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(line);
    }
    flush_paragraph(&mut blocks, &mut paragraph);
    // A retained heading whose entire body consisted of post examples is just
    // visual debris. Drop headings with no content before the next heading.
    let mut useful = vec![true; blocks.len()];
    for (slot, block) in blocks.iter().enumerate() {
        if matches!(block, DefinitionBlock::Heading(_))
            && blocks
                .get(slot + 1)
                .is_none_or(|next| matches!(next, DefinitionBlock::Heading(_)))
        {
            useful[slot] = false;
        }
    }
    blocks
        .into_iter()
        .zip(useful)
        .filter_map(|(block, keep)| keep.then_some(block))
        .collect()
}

fn flush_paragraph(blocks: &mut Vec<DefinitionBlock>, paragraph: &mut String) {
    if paragraph.is_empty() {
        return;
    }
    let text = dtext_inline(paragraph);
    if !text.is_empty() && !example_reference(&text) {
        blocks.push(DefinitionBlock::Paragraph(text));
    }
    paragraph.clear();
}

fn dtext_heading(line: &str) -> Option<&str> {
    let (marker, heading) = line.split_once(". ")?;
    let level = marker.strip_prefix('h')?.split('#').next()?;
    (!level.is_empty() && level.bytes().all(|byte| byte.is_ascii_digit())).then_some(heading)
}

fn omitted_heading(heading: &str) -> bool {
    let heading = heading.trim().to_ascii_lowercase();
    [
        "example",
        "examples",
        "non-example",
        "non-examples",
        "related",
        "related tags",
        "see also",
        "external links",
        "references",
        "resources",
    ]
    .contains(&heading.as_str())
}

fn example_reference(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("!post ") || text.starts_with("!asset ")
}

fn dtext_inline(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("[[") {
        rendered.push_str(&strip_dtext_tags(&rest[..open]));
        let inner = &rest[open + 2..];
        let Some(close) = inner.find("]]") else {
            rendered.push_str(&strip_dtext_tags(&rest[open..]));
            rest = "";
            break;
        };
        let link = &inner[..close];
        let (target, label) = link.split_once('|').unwrap_or((link, link));
        let label = if label.trim().is_empty() {
            target
        } else {
            label
        };
        rendered.push_str(&label.replace('_', " "));
        rest = &inner[close + 2..];
    }
    rendered.push_str(&strip_dtext_tags(rest));
    rendered = rendered
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    rendered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_dtext_tags(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut markup = None;
    for ch in text.chars() {
        match (markup, ch) {
            (None, '<') => markup = Some('>'),
            (None, '[') => markup = Some(']'),
            (Some(end), found) if found == end => markup = None,
            (None, _) => rendered.push(ch),
            (Some(_), _) => {}
        }
    }
    rendered
}

impl TryFrom<DanbooruPost> for Harvest {
    type Error = anyhow::Error;

    fn try_from(post: DanbooruPost) -> Result<Self> {
        let (tags, tag_hints) = tag_inventory(&post);
        let variants = Variants::from(post.media_asset.as_ref());
        let flash = post.file_ext.eq_ignore_ascii_case("swf");
        let (preview_url, thumb_360_url, thumb_720_url, large_url, file_url) = if flash {
            (None, None, None, None, None)
        } else {
            (
                post.preview_file_url.or(variants.thumb_180),
                variants.thumb_360,
                variants.thumb_720,
                post.large_file_url,
                post.file_url,
            )
        };
        let id = narrow_post_id(post.id)?;
        let parent = post.parent_id.map(narrow_post_id).transpose()?;
        let has_children = post.has_children;
        let post = PostRecord {
            id,
            rating: Rating::parse(&post.rating),
            score: post.score,
            favs: post.fav_count,
            width: post.image_width,
            height: post.image_height,
            created_at: post.created_at,
            tags,
            tag_hints,
            preview_url,
            thumb_360_url,
            thumb_720_url,
            large_url,
            file_url,
        };
        Ok(Self {
            post,
            kin: Kin {
                id,
                parent,
                has_children,
            },
        })
    }
}

impl TryFrom<DanbooruKin> for Kin {
    type Error = anyhow::Error;

    fn try_from(post: DanbooruKin) -> Result<Self> {
        Ok(Self {
            id: narrow_post_id(post.id)?,
            parent: post.parent_id.map(narrow_post_id).transpose()?,
            has_children: post.has_children,
        })
    }
}

fn tag_inventory(post: &DanbooruPost) -> (Vec<Tag>, Vec<TagHint>) {
    let mut tags = BTreeSet::new();
    let mut hints = BTreeMap::new();
    for (kind, lane) in [
        (TagKind::General, post.tag_string_general.as_str()),
        (TagKind::Artist, post.tag_string_artist.as_str()),
        (TagKind::Copyright, post.tag_string_copyright.as_str()),
        (TagKind::Character, post.tag_string_character.as_str()),
        (TagKind::Meta, post.tag_string_meta.as_str()),
    ] {
        for tag in lane.split_whitespace().filter_map(Tag::forge) {
            let _inserted = tags.insert(tag.clone());
            let _old = hints.insert(tag, kind);
        }
    }
    for tag in post.tag_string.split_whitespace().filter_map(Tag::forge) {
        let _inserted = tags.insert(tag.clone());
        let _kind = hints.entry(tag).or_insert(TagKind::General);
    }
    let tag_hints = hints
        .into_iter()
        .map(|(tag, kind)| TagHint::new(tag, kind))
        .collect();
    (tags.into_iter().collect(), tag_hints)
}

#[derive(Default)]
struct Variants {
    thumb_180: Option<String>,
    thumb_360: Option<String>,
    thumb_720: Option<String>,
}

impl From<Option<&DanbooruMediaAsset>> for Variants {
    fn from(asset: Option<&DanbooruMediaAsset>) -> Self {
        let mut out = Self::default();
        let Some(asset) = asset else {
            return out;
        };
        for variant in &asset.variants {
            match variant.kind.as_str() {
                "180x180" => out.thumb_180 = Some(variant.url.clone()),
                "360x360" => out.thumb_360 = Some(variant.url.clone()),
                "720x720" => out.thumb_720 = Some(variant.url.clone()),
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dtext_definition_keeps_prose_and_discards_reference_furniture() {
        let blocks = dtext_blocks(
            "Hair that is [[blue hair|blue]].\r\n\r\n\
             h4. Usage\r\n\r\nUse with [[1girl|]].\r\n\r\n\
             * A descriptive bullet\r\n* !post #123: Example\r\n\r\n\
             h4. Examples\r\n\r\n* !post #456\r\n\r\n\
             h4. See also\r\n\r\n* [[aqua hair]]",
        );

        assert_eq!(
            blocks,
            vec![
                DefinitionBlock::Paragraph("Hair that is blue.".to_owned()),
                DefinitionBlock::Heading("Usage".to_owned()),
                DefinitionBlock::Paragraph("Use with 1girl.".to_owned()),
                DefinitionBlock::Bullet("A descriptive bullet".to_owned()),
            ]
        );
    }
}
