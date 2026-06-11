use egui::{Color32, RichText};

use crate::model::{QueryAtom, TagKind};

pub fn text(text: impl Into<String>, kind: TagKind) -> RichText {
    RichText::new(text).color(color(kind))
}

pub fn atom(atom: &QueryAtom, kind: TagKind, negated: bool) -> RichText {
    let text = if negated {
        format!("¬ {atom}")
    } else {
        format!("+ {atom}")
    };
    let text = RichText::new(text).color(color(kind));
    if negated { text.strikethrough() } else { text }
}

pub fn color(kind: TagKind) -> Color32 {
    match kind {
        TagKind::General => Color32::from_rgb(156, 190, 224),
        TagKind::Artist => Color32::from_rgb(222, 184, 116),
        TagKind::Copyright => Color32::from_rgb(190, 153, 220),
        TagKind::Character => Color32::from_rgb(142, 210, 154),
        TagKind::Meta => Color32::from_rgb(213, 151, 123),
    }
}
