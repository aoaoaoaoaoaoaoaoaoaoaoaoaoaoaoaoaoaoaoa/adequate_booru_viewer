//! The showcase: ABV's presentation take over the operator's own mirror,
//! borrowed sealed. A specimen never enters an acceptance verdict.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use abv_contract::{
    Completion, Operator, PRODUCT_NAME, TagRowAction, Target, UI_FINGERPRINT, Water,
    product_directory,
};
use egui_demo::Showcase;
use egui_tester::{
    Anchor, AppCommand, Button, Condition, Error, Graphics, Key, Modifiers, Motion, Network,
    ReactionBudget, Result, Story, StoryObserver, StoryTempo, Testbed, Wheel, WindowQuery, demand,
};

use crate::{FILTER_LIBRARY, Observation, SESSION_STATE, TITLE, session_state};

/// The mirror is paused and sealed; no account exists, so nothing can push.
const CONFIG: &[u8] = b"prefetch_on_hover = true\n\n[mirror]\npolicy = \"paused\"\n";
const GLIDE: Motion = Motion {
    steps: 36,
    duration: Duration::from_millis(520),
};
const DRIFT: Motion = Motion {
    steps: 48,
    duration: Duration::from_millis(800),
};
const FRONTISPIECE_COLUMNS: u16 = 3;
const ARMED: &str = "harmless screenshot";
const SCENERY: &str = "scenery";
/// Tags the score's query already names; the viewer requires another.
const QUERIED: [&str; 8] = [
    "1girl",
    "looking_at_viewer",
    "blue_eyes",
    "red_hair",
    "blue_hair",
    "red_eyes",
    "hat_ribbon",
    "smile",
];

/// Perform the showcase against the specimen world and print every published
/// artifact.
pub fn perform(testbed: &Testbed, binary: &Path, showcase: Showcase) -> Result<()> {
    let specimen = Specimen::admit(testbed)?;
    let app = testbed.launch(specimen.command(binary))?;
    let mut story: Show<'_, '_, Showcase> = Story::bind(
        testbed,
        &app,
        WindowQuery::title_exact(TITLE),
        ReactionBudget::functional(Duration::from_secs(10)),
    )?
    .with_observer(showcase)?
    .with_wait_timeout(Duration::from_secs(90));
    let ready = story.ready(Duration::from_secs(60))?;
    demand(
        ready.state.contract == UI_FINGERPRINT,
        format!(
            "{PRODUCT_NAME} UI contract mismatch: expected {UI_FINGERPRINT}, observed {}",
            ready.state.contract
        ),
    )?;
    score(&mut story)?;
    let showcase = story.finish()?;
    app.terminate()?;
    for artifact in showcase.publish()? {
        println!("{}", artifact.display());
    }
    Ok(())
}

/// The score: water, the Filter Library, the frontispiece, Boolean query
/// algebra, the hover palette, the viewer and its tag drawer, the finale.
fn score<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    story.chapter("Adequate Booru Viewer")?;
    let _answered = story.wait(Condition::new(
        "the sealed mirror answers the armed filter",
        |state: &Observation| state.result_posts > 0,
    ))?;
    let _painted = story.wait(painted())?;
    story.hold(Duration::from_millis(1_200))?;
    water(story)?;
    library(story)?;
    let _query = story
        .tap(Target::Panel("reference-query"), Button::Primary, GLIDE)?
        .until(Condition::new(
            "reference query open",
            |state: &Observation| state.query_open,
        ))?;
    for tile in tiles(story)?.iter().skip(2).take(2) {
        let _wake = story.motion_to(tile.center(), DRIFT)?.next_frame()?;
    }
    let _left = story.leave()?.next_frame()?;
    let _painted = story.wait(painted())?;
    story.still("frontispiece")?;
    story.hold(Duration::from_millis(1_500))?;
    algebra(story)?;
    hover(story)?;
    viewer(story)?;
    finale(story)
}

type Show<'app, 'bed, O> = Story<'app, 'bed, Observation, O>;

fn water<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    story.chapter("Water")?;
    let _ui = story
        .tap(Target::Panel("ui-controls"), Button::Primary, GLIDE)?
        .until(Condition::new("UI recess open", |state: &Observation| {
            state.ui_panel_open
        }))?;
    let _tempted = story
        .point(Target::Water(Water::Dry), GLIDE)?
        .next_frame()?;
    story.hold(Duration::from_millis(500))?;
    let _deluge = story
        .tap(Target::Water(Water::ReallyWet), Button::Primary, GLIDE)?
        .until(water_is("really_wet"))?;
    story.hold(Duration::from_millis(1_800))?;
    let _wet = story
        .tap(Target::Water(Water::Wet), Button::Primary, GLIDE)?
        .until(water_is("wet"))?;
    for tile in tiles(story)?.iter().step_by(5).take(3) {
        let _drift = story.motion_to(tile.center(), DRIFT)?.next_frame()?;
    }
    story.hold(Duration::from_millis(600))?;
    let _closed = story
        .tap(Target::Panel("ui-controls"), Button::Primary, GLIDE)?
        .until(Condition::new("UI recess closed", |state: &Observation| {
            !state.ui_panel_open
        }))?;
    Ok(())
}

/// The density rises to frontispiece scale, then saved Boolean filters
/// switch the whole gallery between worlds.
fn library<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    story.chapter("Filter Library")?;
    let grid = story.anchor(Target::GalleryContent)?.center();
    let _aim = story.motion_to(grid, GLIDE)?.next_frame()?;
    while story.frame()?.state.images_per_row > FRONTISPIECE_COLUMNS {
        let before = story.frame()?.state.images_per_row;
        let _closer = story
            .modified_wheel(grid, -1, Wheel::default(), Modifiers::CTRL)?
            .until(Condition::new(
                "gallery density raised",
                move |state: &Observation| state.images_per_row < before,
            ))?;
        story.hold(Duration::from_millis(400))?;
    }
    let _painted = story.wait(painted())?;
    story.tempo(StoryTempo::TWOFOLD)?;
    let _library = story
        .tap(Target::Panel("filter-library"), Button::Primary, GLIDE)?
        .next_frame()?;
    for shelf in ["work", "play"] {
        let _opened = story
            .tap(Target::Shelf(shelf.into()), Button::Primary, GLIDE)?
            .next_frame()?;
    }
    story.tempo(StoryTempo::STANDARD)?;
    for filter in [SCENERY, ARMED] {
        let _loaded = story
            .tap(Target::Filter(filter.into()), Button::Primary, GLIDE)?
            .until(Condition::new(
                format!("filter `{filter}` armed"),
                move |state: &Observation| state.filter == filter && state.result_posts > 0,
            ))?;
        let _painted = story.wait(painted())?;
        story.hold(Duration::from_millis(1_400))?;
    }
    Ok(())
}

/// Grow both arms of the OR group off live completion, then flip the
/// group's operator to exclusive-or and back.
fn algebra<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    story.chapter("Boolean reference query")?;
    for (arm, stem, tag) in [(1, "hat_r", "hat_ribbon"), (0, "smi", "smile")] {
        let path = vec![0, arm];
        let selected = path.clone();
        let _selected = story
            .tap(Target::Group(path.clone()), Button::Primary, GLIDE)?
            .until(Condition::new(
                format!("group g1.{} selected", arm + 1),
                move |state: &Observation| state.active_group == selected,
            ))?;
        let _entry = story
            .tap(Target::TagEntry, Button::Primary, GLIDE)?
            .until(Condition::new(
                "tag entry focused",
                |state: &Observation| state.text_edit_focused,
            ))?;
        let _typed = story.type_text(stem)?.next_frame()?;
        let candidate = Target::Completion(Completion::Query, tag.into());
        let _offered = story.anchor(&candidate)?;
        story.hold(Duration::from_millis(450))?;
        let _chosen = story
            .tap(&candidate, Button::Primary, GLIDE)?
            .next_frame()?;
        let _atom = story.anchor(Target::Atom(tag.into()))?;
        settle(story)?;
        story.hold(Duration::from_millis(700))?;
    }
    for operator in [Operator::Xor, Operator::Or] {
        let _flipped = story
            .tap(
                Target::GroupOperator(vec![0], operator),
                Button::Primary,
                GLIDE,
            )?
            .next_frame()?;
        settle(story)?;
        story.hold(Duration::from_millis(1_100))?;
    }
    Ok(())
}

fn hover<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    story.chapter("Hover")?;
    for tile in tiles(story)?.iter().take(2) {
        let _lift = story.motion_to(tile.center(), DRIFT)?.next_frame()?;
        story.hold(Duration::from_millis(500))?;
    }
    let grid = story.anchor(Target::GalleryContent)?.center();
    let _down = story.scroll(grid, 4)?.next_frame()?;
    story.hold(Duration::from_millis(900))?;
    let _up = story.scroll(grid, -4)?.next_frame()?;
    let _painted = story.wait(painted())?;
    Ok(())
}

fn viewer<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    story.chapter("Viewer")?;
    let tile = tiles(story)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Verdict {
            detail: "the gallery offered no tile to open".to_owned(),
        })?;
    let _opened = story
        .tap(&tile.name, Button::Primary, GLIDE)?
        .until(Condition::new(
            "reference image open",
            |state: &Observation| state.viewer_post.is_some(),
        ))?;
    let surface = Target::ViewerSurface.wire();
    let settled = story.wait_stable(
        Duration::from_secs(10),
        Duration::from_millis(250),
        "viewer window geometry",
        |frame| frame.anchor(&surface).map(|anchor| anchor.rect),
    )?;
    let [left, top, right, bottom] = settled
        .anchor(&surface)
        .ok_or_else(|| Error::Verdict {
            detail: "the open viewer published no image surface".to_owned(),
        })?
        .rect;
    for (x, y) in [(0.38, 0.42), (0.6, 0.58)] {
        let point = (
            (left + (right - left) * x) as i16,
            (top + (bottom - top) * y) as i16,
        );
        let _ring = story.motion_to(point, GLIDE)?.next_frame()?;
        let _splash = story.session().click(point.0, point.1, Button::Primary)?;
        story.hold(Duration::from_millis(900))?;
    }
    story.still("viewer")?;
    let _drawer = story
        .key(Key::Character('t'))?
        .until(Condition::new("tag drawer open", |state: &Observation| {
            state.viewer_tags_open
        }))?;
    let rows = Target::TagRow(TagRowAction::Require, "".into()).wire();
    let require = story
        .frame()?
        .anchors
        .iter()
        .map(|anchor| anchor.name.to_string())
        .find(|name| {
            name.strip_prefix(rows.as_ref())
                .is_some_and(|tag| !QUERIED.contains(&tag))
        });
    if let Some(require) = require {
        let _read = story.point(&require, GLIDE)?.next_frame()?;
        story.hold(Duration::from_millis(800))?;
        let _required = story.tap(&require, Button::Primary, GLIDE)?.next_frame()?;
        story.hold(Duration::from_millis(900))?;
    }
    let _closed = story
        .key(Key::Escape)?
        .until(Condition::new("viewer closed", |state: &Observation| {
            state.viewer_post.is_none()
        }))?;
    settle(story)
}

fn finale<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    story.chapter("Finale")?;
    story.tempo(StoryTempo::TWOFOLD)?;
    let _folded = story
        .tap(Target::Panel("reference-query"), Button::Primary, GLIDE)?
        .next_frame()?;
    let _gallery = story
        .tap(Target::Panel("gallery-controls"), Button::Primary, GLIDE)?
        .next_frame()?;
    for sort in ["newest", "score"] {
        let _sorted = story
            .tap(Target::Sort(sort), Button::Primary, GLIDE)?
            .next_frame()?;
        settle(story)?;
        story.hold(Duration::from_millis(900))?;
    }
    story.tempo(StoryTempo::STANDARD)?;
    let grid = story.anchor(Target::GalleryContent)?.center();
    let _aim = story.motion_to(grid, GLIDE)?.next_frame()?;
    let _plunge = story.scroll(grid, 14)?.next_frame()?;
    story.hold(Duration::from_millis(2_400))
}

/// Wait out a result change: the count holds still and every tile paints.
fn settle<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<()> {
    let _stable = story.wait_stable(
        Duration::from_secs(10),
        Duration::from_millis(300),
        "result count",
        |frame| Some(frame.state.result_posts),
    )?;
    let _painted = story.wait(painted())?;
    Ok(())
}

/// Gallery tiles wholly inside the visible grid, in reading order.
fn tiles<O: StoryObserver>(story: &mut Show<'_, '_, O>) -> Result<Vec<Anchor>> {
    let frame = story.frame()?;
    let content = frame
        .anchor(&Target::GalleryContent.wire())
        .ok_or_else(|| Error::Verdict {
            detail: "the gallery published no content rectangle".to_owned(),
        })?
        .rect;
    let mut tiles = frame
        .anchors
        .iter()
        .filter(|anchor| {
            let name = anchor.name.as_ref();
            name.rsplit_once('/')
                .and_then(|(_, id)| id.parse().ok())
                .is_some_and(|id| Target::Tile(id).wire() == name)
                && anchor.rect[1] >= content[1]
                && anchor.rect[3] <= content[3]
        })
        .cloned()
        .collect::<Vec<_>>();
    tiles.sort_by(|a, b| {
        a.rect[1]
            .total_cmp(&b.rect[1])
            .then(a.rect[0].total_cmp(&b.rect[0]))
    });
    Ok(tiles)
}

fn painted() -> Condition<Observation> {
    Condition::new(
        "every visible gallery tile painted",
        |state: &Observation| state.tiles.visible > 0 && state.tiles.painted == state.tiles.visible,
    )
}

fn water_is(expected: &'static str) -> Condition<Observation> {
    Condition::new(
        format!("water mode {expected}"),
        move |state: &Observation| state.water == expected,
    )
}

/// The operator's mirror, admitted read-only: the index and its atlas are
/// borrowed sealed, and the media cache is copied private. Configuration,
/// filters, and session state are fixtures; favorites and credentials never
/// enter.
struct Specimen {
    index: PathBuf,
    atlas: Option<PathBuf>,
}

impl Specimen {
    fn admit(testbed: &Testbed) -> Result<Self> {
        let product = product_directory();
        let index = operator_root("XDG_DATA_HOME", ".local/share")?
            .join(product)
            .join("index.redb");
        demand(
            index.is_file(),
            format!("no specimen index at {}", index.display()),
        )?;
        let atlas = Some(index.with_extension("kin.u32")).filter(|atlas| atlas.is_file());
        let media = operator_root("XDG_CACHE_HOME", ".cache")?
            .join(product)
            .join("media");
        let _media = testbed.create_private_dir(format!("xdg/cache/{product}/media"))?;
        for entry in fs::read_dir(&media).map_err(|source| Error::Io {
            operation: "list specimen media cache",
            path: media.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                operation: "read specimen media cache",
                path: media.clone(),
                source,
            })?;
            let path = entry.path();
            if path.is_file() {
                let _copy = testbed.copy_private(
                    Path::new("xdg/cache")
                        .join(product)
                        .join("media")
                        .join(entry.file_name()),
                    &path,
                )?;
            }
        }
        let _config = testbed.write_private(format!("xdg/config/{product}/config.toml"), CONFIG)?;
        let _filters =
            testbed.write_private(format!("xdg/data/{product}/filters.toml"), FILTER_LIBRARY)?;
        let _session = testbed.write_private(session_state().as_str(), SESSION_STATE)?;
        Ok(Self { index, atlas })
    }

    fn command(&self, binary: &Path) -> AppCommand {
        let command = AppCommand::new(binary)
            .arg("--sealed-index")
            .arg(&self.index)
            .borrow_read_only(&self.index)
            .graphics(Graphics::Host)
            .network(Network::Host)
            .runtime(Duration::from_secs(600))
            .witness("probes/abv.observations");
        match &self.atlas {
            Some(atlas) => command.borrow_read_only(atlas),
            None => command,
        }
    }
}

fn operator_root(variable: &str, fallback: &str) -> Result<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .ok_or_else(|| Error::Verdict {
            detail: format!("neither {variable} nor HOME locates the operator's mirror"),
        })
}
