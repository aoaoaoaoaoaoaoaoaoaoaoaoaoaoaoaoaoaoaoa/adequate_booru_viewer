use super::*;
use crate::model::{GalleryTopology, Harvest, Kin, Rating, TagHint, TagKind, TagPolarity};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn search_tail_proves_exhaustion_without_a_silent_horizon() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-search-tail-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    remove_index(&path);
    let index = Index::open(&path)?;
    index.absorb(&[
        post(1, 10, Rating::General, &["solo"])?,
        post(2, 20, Rating::General, &["solo"])?,
        post(3, 30, Rating::General, &["solo"])?,
    ])?;
    let query = Query::parse("solo");

    for sort in Sort::ALL {
        let empty = index.search(&query, sort, DateRange::default(), 0)?;
        assert!(empty.posts.is_empty());
        assert_eq!(empty.candidates, 3);
        assert_eq!(empty.horizon, 0);
        assert_eq!(empty.tail, SearchTail::Exhausted);
    }

    let shallow = index.search(&query, Sort::Score, DateRange::default(), 2)?;
    assert_eq!(shallow.horizon, 2);
    assert_eq!(shallow.tail, SearchTail::Open);
    assert_eq!(ids(shallow), [3, 2]);

    let deep = index.search(&query, Sort::Score, DateRange::default(), 4)?;
    assert_eq!(deep.horizon, 4);
    assert_eq!(deep.tail, SearchTail::Exhausted);
    assert_eq!(ids(deep), [3, 2, 1]);

    drop(index);
    remove_index(&path);
    Ok(())
}

#[test]
fn boolean_query_evaluator_matches_its_truth_algebra() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-bool-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    remove_index(&path);
    let index = Index::open(&path)?;
    let records = (0_u8..8)
        .map(|mask| {
            let tags = [("a", 1), ("b", 2), ("c", 4)]
                .into_iter()
                .filter_map(|(tag, bit)| (mask & bit != 0).then_some(tag))
                .collect::<Vec<_>>();
            post(u32::from(mask) + 1, i32::from(mask), Rating::General, &tags)
        })
        .collect::<Result<Vec<_>>>()?;
    index.absorb(&records)?;

    let and = Query::parse("a b");
    assert_truth(&index, &and, |mask| mask & 1 != 0 && mask & 2 != 0)?;

    let mut or = Query::default();
    let choice = or.push_group(&[], BoolOp::Or).context("push OR")?;
    assert!(or.push_atom(&choice, atom("a")?, TagPolarity::Positive));
    assert!(or.push_atom(&choice, atom("b")?, TagPolarity::Positive));
    assert_truth(&index, &or, |mask| mask & 1 != 0 || mask & 2 != 0)?;

    let mut xor = Query::default();
    let choice = xor.push_group(&[], BoolOp::Xor).context("push XOR")?;
    assert!(xor.push_atom(&choice, atom("a")?, TagPolarity::Positive));
    assert!(xor.push_atom(&choice, atom("b")?, TagPolarity::Positive));
    assert!(xor.push_atom(&choice, atom("c")?, TagPolarity::Positive));
    assert_truth(&index, &xor, u8::is_power_of_two)?;

    let mut neither = or;
    assert!(neither.toggle_not(&choice));
    assert_truth(&index, &neither, |mask| mask.trailing_zeros() >= 2)?;

    drop(index);
    remove_index(&path);
    Ok(())
}

#[test]
fn absorption_is_query_visible_across_merge_replacement_and_retraction() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-facts-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    remove_index(&path);
    let index = Index::open(&path)?;
    index.absorb(&[
        post(1, 10, Rating::General, &["solo", "bikini"])?,
        post(2, 20, Rating::General, &["solo"])?,
    ])?;

    let solo = Query::parse("solo");
    assert_eq!(
        ids(index.search(&solo, Sort::Newest, DateRange::default(), 10)?),
        [2, 1]
    );
    assert_eq!(index.stats()?.pending_fact_batches, 1);

    let merge = index.merge_pending_facts(FactMergeBudget {
        batches: 16,
        bytes: usize::MAX,
    })?;
    assert_eq!(merge.batches, 1);
    assert_eq!(
        ids(index.search(&solo, Sort::Score, DateRange::default(), 10)?),
        [2, 1]
    );
    assert_eq!(index.stats()?.pending_fact_batches, 0);

    index.absorb(&[post(2, 30, Rating::General, &["bikini"])?])?;
    assert_eq!(
        ids(index.search(&solo, Sort::Newest, DateRange::default(), 10)?),
        [1]
    );

    assert_eq!(
        ids(index.search(
            &Query::parse("bikini"),
            Sort::Score,
            DateRange::default(),
            10
        )?),
        [2, 1]
    );
    let merge = index.merge_pending_facts(FactMergeBudget {
        batches: 16,
        bytes: usize::MAX,
    })?;
    assert_eq!(merge.batches, 1);
    assert_eq!(
        ids(index.search(&solo, Sort::Newest, DateRange::default(), 10)?),
        [1]
    );

    let mut forbidden = post(1, 40, Rating::General, &["solo"])?;
    forbidden.file_url = Some("https://example.test/1.swf".to_owned());
    index.absorb(&[forbidden])?;
    assert!(
        index
            .search(&solo, Sort::Newest, DateRange::default(), 10)?
            .posts
            .is_empty()
    );
    let _merged = index.merge_pending_facts(FactMergeBudget {
        batches: 16,
        bytes: usize::MAX,
    })?;
    assert!(
        index
            .search(&solo, Sort::Newest, DateRange::default(), 10)?
            .posts
            .is_empty()
    );

    drop(index);
    remove_index(&path);
    Ok(())
}

#[test]
fn tag_kind_hints_are_durable() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-tag-kind-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    remove_index(&path);
    let index = Index::open(&path)?;
    let idol = Tag::forge("idol").context("idol tag")?;
    let mut post = post(7, 10, Rating::General, &["idol"])?;
    post.tag_hints = vec![TagHint::new(idol.clone(), TagKind::Character)];
    index.absorb(&[post])?;
    assert_eq!(index.tag_kind(&idol)?, TagKind::Character);
    assert_eq!(
        index
            .tag_suggestions("id", 1)?
            .first()
            .map(|suggestion| suggestion.kind),
        Some(TagKind::Character)
    );
    drop(index);
    let index = Index::open(&path)?;
    assert_eq!(index.tag_kind(&idol)?, TagKind::Character);
    assert_eq!(
        index
            .tag_suggestions("id", 1)?
            .first()
            .map(|suggestion| suggestion.kind),
        Some(TagKind::Character)
    );
    drop(index);
    remove_index(&path);
    Ok(())
}

#[test]
fn date_range_compiles_to_chronological_id_window() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-dates-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    remove_index(&path);
    let index = Index::open(&path)?;
    index.absorb(&[
        dated_post(10, 10, "2024-01-01", &["solo"])?,
        dated_post(20, 40, "2024-06-01", &["solo"])?,
        dated_post(30, 30, "2024-06-02", &["solo"])?,
        dated_post(40, 50, "2025-01-01", &["solo"])?,
    ])?;
    let dates = DateRange {
        first: CreatedDay::parse("2024-06-01"),
        last: CreatedDay::parse("2024-12-31"),
    };
    let solo = Query::parse("solo");
    assert_eq!(ids(index.search(&solo, Sort::Newest, dates, 10)?), [30, 20]);
    assert_eq!(ids(index.search(&solo, Sort::Score, dates, 10)?), [20, 30]);

    drop(index);
    remove_index(&path);
    Ok(())
}

#[test]
fn family_projection_keeps_the_strongest_matching_member() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-families-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let index = Index::open(&path)?;
    index.absorb_harvest(&[
        harvest(post(1, 10, Rating::General, &["solo"])?, None, true),
        harvest(post(2, 100, Rating::General, &["solo"])?, Some(1), false),
        harvest(post(3, 50, Rating::General, &["solo"])?, Some(1), false),
        harvest(post(4, 80, Rating::General, &["solo"])?, None, false),
    ])?;
    let solo = Query::parse("solo");
    let grouped = index.search_topology(
        &solo,
        &Arc::new(RoaringBitmap::new()),
        Sort::Score,
        DateRange::default(),
        GalleryTopology::Grouped,
        10,
    )?;
    assert_eq!(ids(grouped.clone()), [2, 4]);
    assert_eq!(
        grouped.families.get(&PostId(2)).map(|badge| badge.posts),
        Some(3)
    );
    assert_eq!(
        ids(index.search_topology(
            &solo,
            &Arc::new(RoaringBitmap::new()),
            Sort::Newest,
            DateRange::default(),
            GalleryTopology::Grouped,
            10,
        )?),
        [4, 3]
    );
    assert!(
        index
            .search_topology(
                &solo,
                &Arc::new(RoaringBitmap::new()),
                Sort::Newest,
                DateRange::default(),
                GalleryTopology::Grouped,
                0,
            )?
            .posts
            .is_empty()
    );
    assert_eq!(
        ids(index.search(&solo, Sort::Score, DateRange::default(), 10)?),
        [2, 4, 3, 1]
    );
    let tree = index.family_tree(PostId(3))?;
    assert_eq!(tree.root, PostId(1));
    assert_eq!(
        tree.node(PostId(1)).map(|node| node.children.as_slice()),
        Some(&[PostId(2), PostId(3)][..])
    );
    assert!(!index.family_hydrated(PostId(2))?);
    index.absorb_family(
        &[
            Kin {
                id: PostId(1),
                parent: None,
                has_children: true,
            },
            Kin {
                id: PostId(2),
                parent: Some(PostId(1)),
                has_children: false,
            },
            Kin {
                id: PostId(3),
                parent: Some(PostId(1)),
                has_children: false,
            },
        ],
        PostId(1),
    )?;
    assert!(index.family_hydrated(PostId(2))?);

    index.absorb_harvest(&[
        harvest(post(10, 1, Rating::General, &["animated"])?, None, true),
        harvest(
            post(11, 20, Rating::General, &["hidden_child"])?,
            Some(10),
            false,
        ),
        harvest(
            post(12, 30, Rating::General, &["hidden_child"])?,
            Some(10),
            false,
        ),
    ])?;
    let hidden = index.search_topology(
        &Query::parse("hidden_child"),
        &Arc::new(RoaringBitmap::new()),
        Sort::Score,
        DateRange::default(),
        GalleryTopology::Grouped,
        10,
    )?;
    assert_eq!(ids(hidden), [12]);
    let hidden_tree = index.family_tree(PostId(12))?;
    assert!(
        hidden_tree
            .node(PostId(10))
            .is_some_and(|node| node.post.is_none())
    );
    assert_eq!(hidden_tree.badge().map(|badge| badge.posts), Some(2));

    drop(index);
    let reopened = Index::open(&path)?;
    assert_eq!(reopened.family_tree(PostId(2))?.root, PostId(1));
    assert!(reopened.family_hydrated(PostId(3))?);
    assert_eq!(reopened.family_tree(PostId(12))?.root, PostId(10));
    drop(reopened);
    remove_index(&path);
    Ok(())
}

#[test]
fn reparenting_amends_both_reverse_edges_and_the_atlas() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-reparent-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let index = Index::open(&path)?;
    index.absorb_harvest(&[
        harvest(post(20, 10, Rating::General, &["solo"])?, None, true),
        harvest(post(21, 30, Rating::General, &["solo"])?, Some(20), false),
        harvest(post(30, 20, Rating::General, &["solo"])?, None, false),
    ])?;
    assert_eq!(index.family_tree(PostId(21))?.root, PostId(20));

    index.absorb_harvest(&[harvest(
        post(21, 30, Rating::General, &["solo"])?,
        Some(30),
        false,
    )])?;
    assert_eq!(index.family_tree(PostId(21))?.root, PostId(30));
    assert!(
        index
            .family_tree(PostId(20))?
            .node(PostId(20))
            .is_some_and(|node| node.children.is_empty())
    );
    assert_eq!(
        index
            .family_tree(PostId(30))?
            .node(PostId(30))
            .map(|node| node.children.as_slice()),
        Some(&[PostId(21)][..])
    );

    drop(index);
    remove_index(&path);
    Ok(())
}

#[test]
fn local_favorites_prefix_every_sort_without_escaping_the_query() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "adequate-booru-local-favorites-{}.redb",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let index = Index::open(&path)?;
    let mut root = post(1, 1, Rating::General, &["solo"])?;
    root.favs = 1;
    let mut child = post(2, 100, Rating::General, &["solo"])?;
    child.favs = 100;
    let mut peer = post(3, 50, Rating::General, &["solo"])?;
    peer.favs = 50;
    let outsider = post(4, 200, Rating::General, &["landscape"])?;
    index.absorb_harvest(&[
        harvest(root, None, true),
        harvest(child, Some(1), false),
        harvest(peer, None, false),
        harvest(outsider, None, false),
    ])?;
    let favorites = Arc::new([1_u32, 4, 999].into_iter().collect());
    let solo = Query::parse("solo");

    assert_eq!(
        ids(index.search_topology(
            &Query::default(),
            &favorites,
            Sort::Score,
            DateRange::default(),
            GalleryTopology::Ungrouped,
            10,
        )?),
        [4, 1, 2, 3]
    );
    assert!(
        lock(&index.sort_keys).get(Sort::Score).is_none(),
        "favorite ordering must not materialize the dense id→rank projection"
    );

    for (sort, expected) in [
        (Sort::Newest, [1, 3, 2]),
        (Sort::Score, [1, 2, 3]),
        (Sort::FavCount, [1, 2, 3]),
    ] {
        assert_eq!(
            ids(index.search_topology(
                &solo,
                &favorites,
                sort,
                DateRange::default(),
                GalleryTopology::Ungrouped,
                10,
            )?),
            expected
        );
    }

    let hit = index.search_corpus(
        &Query::default(),
        &favorites,
        Corpus::LocalFavorites,
        Sort::Score,
        DateRange::default(),
        GalleryTopology::Ungrouped,
        10,
    )?;
    assert_eq!(ids(hit.clone()), [4, 1]);
    assert_eq!(hit.candidates, 2);

    assert_eq!(
        ids(index.search_topology(
            &solo,
            &favorites,
            Sort::Score,
            DateRange::default(),
            GalleryTopology::Grouped,
            10,
        )?),
        [1, 3]
    );

    drop(index);
    remove_index(&path);
    Ok(())
}

fn ids(hit: SearchHit) -> Vec<u32> {
    hit.posts.into_iter().map(|post| post.id.0).collect()
}

fn assert_truth(index: &Index, query: &Query, law: impl Fn(u8) -> bool) -> Result<()> {
    let expected = (0_u8..8)
        .rev()
        .filter(|mask| law(*mask))
        .map(|mask| u32::from(mask) + 1)
        .collect::<Vec<_>>();
    assert_eq!(
        ids(index.search(query, Sort::Newest, DateRange::default(), 8)?),
        expected
    );
    Ok(())
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
        tag_hints: Vec::new(),
        preview_url: Some(format!("https://example.test/{id}.jpg")),
        thumb_360_url: None,
        thumb_720_url: None,
        large_url: None,
        file_url: None,
    })
}

fn dated_post(id: u32, score: i32, created: &str, tags: &[&str]) -> Result<PostRecord> {
    Ok(PostRecord {
        created_at: format!("{created}T00:00:00Z"),
        ..post(id, score, Rating::General, tags)?
    })
}

fn harvest(post: PostRecord, parent: Option<u32>, has_children: bool) -> Harvest {
    let id = post.id;
    Harvest {
        post,
        kin: Kin {
            id,
            parent: parent.map(PostId),
            has_children,
        },
    }
}

fn remove_index(path: &Path) {
    let _removed = std::fs::remove_file(path);
    let _removed = std::fs::remove_file(path.with_extension("kin.u32"));
}
