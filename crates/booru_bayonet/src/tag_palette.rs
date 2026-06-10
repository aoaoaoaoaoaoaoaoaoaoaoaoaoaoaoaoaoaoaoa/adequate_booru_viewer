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
    TagKind::ALL
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PostId, Rating, TagHint};
    use anyhow::{Context as _, Result};

    #[test]
    fn menu_groups_artist_and_character_tags() -> Result<()> {
        let artist = Tag::forge("ciloranko").context("artist tag")?;
        let character = Tag::forge("hakurei_reimu").context("character tag")?;
        let general = Tag::forge("solo").context("general tag")?;
        let post = PostRecord {
            id: PostId(1),
            rating: Rating::General,
            score: 0,
            favs: 0,
            width: 1,
            height: 1,
            created_at: String::new(),
            tags: vec![artist.clone(), character.clone(), general.clone()],
            tag_hints: vec![
                TagHint::new(artist.clone(), TagKind::Artist),
                TagHint::new(character.clone(), TagKind::Character),
                TagHint::new(general.clone(), TagKind::General),
            ],
            preview_url: None,
            thumb_360_url: None,
            thumb_720_url: None,
            large_url: None,
            file_url: None,
        };

        let groups = grouped(&post, |_| TagKind::General);

        assert_eq!(bucket(&groups, TagKind::Artist), vec![artist]);
        assert_eq!(bucket(&groups, TagKind::Character), vec![character]);
        assert_eq!(bucket(&groups, TagKind::General), vec![general]);
        Ok(())
    }

    fn bucket(groups: &[(TagKind, Vec<Tag>)], kind: TagKind) -> Vec<Tag> {
        groups
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map_or_else(Vec::new, |(_, tags)| tags.clone())
    }
}
