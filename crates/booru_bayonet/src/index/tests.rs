use super::*;
use crate::model::{Rating, TagPolarity};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn boolean_query_evaluator_cuts_with_roaring_algebra() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "booru-bayonet-bool-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let _stale = std::fs::remove_file(&path);
    let index = Index::open(&path)?;
    index.absorb(&[
        post(1, 10, Rating::General, &["solo", "bikini"])?,
        post(2, 20, Rating::Questionable, &["solo", "nude"])?,
        post(3, 30, Rating::Explicit, &["bikini", "nude"])?,
        post(4, 40, Rating::Sensitive, &["solo"])?,
        post(5, 50, Rating::General, &["bikini", "nude", "swimsuit"])?,
        post(6, 60, Rating::General, &["swimsuit"])?,
    ])?;

    let mut query = Query::default();
    assert!(query.push_atom(&[], atom("solo")?, TagPolarity::Positive));
    let choice = query.push_group(&[], BoolOp::Or).context("push OR")?;
    assert!(query.push_atom(&choice, atom("bikini")?, TagPolarity::Positive));
    assert!(query.push_atom(&choice, atom("nude")?, TagPolarity::Positive));
    assert!(query.push_atom(
        &[],
        QueryAtom::Rating(RatingClass::Explicit),
        TagPolarity::Negative
    ));
    assert_eq!(ids(index.search(&query, Sort::Score, 10)?), [2, 1]);

    let mut xor = Query::default();
    let choice = xor.push_group(&[], BoolOp::Xor).context("push XOR")?;
    assert!(xor.push_atom(&choice, atom("bikini")?, TagPolarity::Positive));
    assert!(xor.push_atom(&choice, atom("nude")?, TagPolarity::Positive));
    assert!(xor.push_atom(&choice, atom("swimsuit")?, TagPolarity::Positive));
    assert_eq!(ids(index.search(&xor, Sort::Newest, 10)?), [6, 2, 1]);

    drop(index);
    let _removed = std::fs::remove_file(&path);
    Ok(())
}

#[test]
fn posting_facts_are_query_visible_before_and_after_chunk_merge() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "booru-bayonet-facts-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let _stale = std::fs::remove_file(&path);
    let index = Index::open(&path)?;
    index.absorb(&[
        post(1, 10, Rating::General, &["solo", "bikini"])?,
        post(2, 20, Rating::General, &["solo"])?,
    ])?;

    let solo = Query::parse("solo");
    assert_eq!(ids(index.search(&solo, Sort::Newest, 10)?), [2, 1]);
    assert_eq!(index.stats()?.pending_fact_batches, 1);

    let merge = index.merge_pending_facts(FactMergeBudget {
        batches: 16,
        bytes: usize::MAX,
    })?;
    assert_eq!(merge.batches, 1);
    assert_eq!(ids(index.search(&solo, Sort::Score, 10)?), [2, 1]);
    assert_eq!(index.stats()?.pending_fact_batches, 0);

    index.absorb(&[post(2, 30, Rating::General, &["bikini"])?])?;
    assert_eq!(ids(index.search(&solo, Sort::Newest, 10)?), [1]);
    assert_eq!(
        ids(index.search(&Query::parse("bikini"), Sort::Score, 10)?),
        [2, 1]
    );
    let merge = index.merge_pending_facts(FactMergeBudget {
        batches: 16,
        bytes: usize::MAX,
    })?;
    assert_eq!(merge.batches, 1);
    assert_eq!(ids(index.search(&solo, Sort::Newest, 10)?), [1]);

    drop(index);
    let _removed = std::fs::remove_file(&path);
    Ok(())
}

fn ids(hit: SearchHit) -> Vec<u32> {
    hit.posts.into_iter().map(|post| post.id.0).collect()
}

fn atom(raw: &str) -> Result<QueryAtom> {
    Tag::forge(raw)
        .map(QueryAtom::Tag)
        .context("forge test tag")
}

fn post(id: u32, score: i32, rating: Rating, tags: &[&str]) -> Result<PostRecord> {
    Ok(PostRecord {
        id: PostId(id),
        rating,
        score,
        favs: 0,
        width: 1,
        height: 1,
        created_at: String::new(),
        tags: tags
            .iter()
            .map(|tag| Tag::forge(tag).context("forge post tag"))
            .collect::<Result<Vec<_>>>()?,
        preview_url: None,
        thumb_360_url: None,
        thumb_720_url: None,
        large_url: None,
        file_url: None,
    })
}
