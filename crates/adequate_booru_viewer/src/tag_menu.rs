use crate::model::{PostId, PostRecord, Tag, TagKind};

pub const HEIGHT: f32 = 360.0;
pub const WIDTH: f32 = 320.0;

const MARGIN: f32 = 8.0;

pub type TagGroups = Vec<(TagKind, Vec<Tag>)>;

#[derive(Clone, Debug, Default)]
pub enum TagMenu {
    #[default]
    Closed,
    Open {
        post: Box<PostRecord>,
        anchor: egui::Pos2,
        /// Tag groups resolved once at open, so painting the palette costs no index reads.
        groups: TagGroups,
    },
}

impl TagMenu {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }

    pub fn post_id(&self) -> Option<PostId> {
        match self {
            Self::Closed => None,
            Self::Open { post, .. } => Some(post.as_ref().id),
        }
    }

    pub fn view(&self) -> Option<(&PostRecord, egui::Pos2, &TagGroups)> {
        match self {
            Self::Closed => None,
            Self::Open {
                post,
                anchor,
                groups,
            } => Some((post.as_ref(), *anchor, groups)),
        }
    }
}

pub fn position(anchor: egui::Pos2, screen: egui::Rect) -> egui::Pos2 {
    let limit = egui::vec2(WIDTH, HEIGHT + 80.0);
    egui::pos2(
        clamp(anchor.x, screen.min.x + MARGIN, screen.max.x - limit.x),
        clamp(anchor.y, screen.min.y + MARGIN, screen.max.y - limit.y),
    )
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if max < min {
        min
    } else {
        value.clamp(min, max)
    }
}
