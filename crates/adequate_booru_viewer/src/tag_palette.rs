use crate::model::{PostRecord, Tag, TagKind};

pub fn grouped(
    post: &PostRecord,
    mut learned: impl FnMut(&Tag) -> TagKind,
) -> Vec<(TagKind, Vec<Tag>)> {
    let tags = post
        .tags
        .iter()
        .map(|tag| {
            let kind = match post.tag_kind(tag) {
                TagKind::General => learned(tag),
                kind => kind,
            };
            (tag.clone(), kind)
        })
        .collect::<Vec<_>>();
    TagKind::PALETTE_ORDER
        .into_iter()
        .filter_map(|kind| {
            let group = tags
                .iter()
                .filter(|(_, tag_kind)| *tag_kind == kind)
                .map(|(tag, _)| tag.clone())
                .collect::<Vec<_>>();
            (!group.is_empty()).then_some((kind, group))
        })
        .collect()
}
