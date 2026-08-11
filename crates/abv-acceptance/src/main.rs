use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use adequate_booru_viewer::{
    index::Index,
    model::{PostId, PostRecord, Rating, Tag},
};
use egui_tester::{
    AppCommand, Application, Backend, Button, Condition, Error, Frame, Graphics, Key, Modifiers,
    Network, PixelRegion, Probe, ReactionBudget, Result, Story, Testbed, TestbedBuilder,
    WaylandConfig, WindowQuery, X11Config, demand,
};
use serde::Deserialize;

const TITLE: &str = "adequate booru viewer";
const SLATE: &str = "xdg/state/adequate_booru_viewer/slate.toml";
const BROWSER_RECORD: &str = "effects/danbooru-url";
const EFFECT_POST: u32 = 9_000_001;
const DEMO_CONFIG: &[u8] = include_bytes!("../../../demo/wet/config.toml");
const DEMO_SLATE: &[u8] = include_bytes!("../../../demo/wet/slate.toml");

fn main() -> Result<()> {
    match env::args().nth(1).as_deref() {
        Some("--read-clipboard") => return print_clipboard(),
        Some("--record-browser") => return record_browser(),
        _ => {}
    }
    let cli = Cli::parse()?;
    let helper = acceptance_executable()?;
    let binary = env::var_os("ABV_ACCEPTANCE_BINARY")
        .map(PathBuf::from)
        .map_or_else(|| sibling_binary(&helper), Ok)?;
    let artifacts = cli
        .artifacts
        .or_else(|| env::var_os("ABV_ACCEPTANCE_ARTIFACTS").map(PathBuf::from));
    let mut builder = TestbedBuilder::default().backend(cli.backend);
    if let Some(artifacts) = &artifacts {
        builder = builder.failure_artifacts(artifacts);
    }
    builder.run(|testbed| {
        seed(testbed)?;
        let harness = Harness {
            testbed,
            binary: &binary,
            helper: &helper,
            artifacts: artifacts.as_deref(),
        };
        if cli.smoke {
            smoke(&harness, cli.backend)
        } else {
            keyboard_contract(&harness)?;
            water_persists(&harness)?;
            native_effects(&harness)?;
            println!("abv acceptance passed under {}", harness.testbed.id());
            Ok(())
        }
    })
}

#[derive(Debug, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the wire observation carries independent UI facts, not a latent state machine"
)]
struct Observation {
    contract: String,
    water: String,
    filter: String,
    result_posts: usize,
    text_edit_focused: bool,
    ui_open: bool,
    query_open: bool,
    active_group: Vec<usize>,
    images_per_row: u16,
    guide_open: bool,
    viewer_tags_open: bool,
}

type AbvStory<'app, 'bed> = Story<'app, 'bed, Observation>;

struct Harness<'a> {
    testbed: &'a Testbed,
    binary: &'a Path,
    helper: &'a Path,
    artifacts: Option<&'a Path>,
}

impl<'a> Harness<'a> {
    fn command(&self, witnessed: bool) -> AppCommand {
        let browser = format!("{} --record-browser", self.helper.display());
        let command = AppCommand::new(self.binary)
            .borrow_read_only(self.helper)
            .env("BROWSER", browser)
            .graphics(Graphics::Software)
            .network(Network::Deny)
            .runtime(Duration::from_secs(45));
        if witnessed {
            command.witness("probes/abv.observations")
        } else {
            command
        }
    }

    fn launch(&self, witnessed: bool) -> Result<Application<'a>> {
        self.testbed.launch(self.command(witnessed))
    }

    fn story<'app>(&'a self, app: &'app Application<'a>) -> Result<AbvStory<'app, 'a>> {
        let mut story: AbvStory<'app, 'a> = Story::bind(
            self.testbed,
            app,
            WindowQuery::title_exact(TITLE),
            ReactionBudget::functional(Duration::from_secs(5)),
        )?;
        let ready = story.ready(Duration::from_secs(15))?;
        demand(
            ready.state.contract == abv_contract::UI_FINGERPRINT,
            format!(
                "ABV UI contract mismatch: expected {}, observed {}",
                abv_contract::UI_FINGERPRINT,
                ready.state.contract
            ),
        )?;
        Ok(story)
    }
}

fn smoke(harness: &Harness<'_>, backend: Backend) -> Result<()> {
    match backend {
        Backend::X11(_) => smoke_x11(harness),
        Backend::Wayland(_) => smoke_wayland(harness),
    }
}

fn smoke_x11(harness: &Harness<'_>) -> Result<()> {
    let app = harness.launch(false)?;
    let session = harness.testbed.x11_session(
        &app,
        WindowQuery::title_exact(TITLE),
        Duration::from_secs(15),
    )?;
    session.focus()?;
    let first = session.capture()?;
    let frame = if visible(&first) {
        first
    } else {
        session.wait_changed(&first, 0.001, 2, Duration::from_secs(15))?
    };
    demand(
        visible(&frame),
        "uninstrumented ABV rendered only black pixels",
    )?;
    app.terminate()
}

fn smoke_wayland(harness: &Harness<'_>) -> Result<()> {
    let app = harness.launch(true)?;
    let mut witness = app.witness()?.typed::<Observation>();
    let presented = witness.wait_surface_presented(&app, Duration::from_secs(30))?;
    demand(
        presented.state.contract == abv_contract::UI_FINGERPRINT,
        format!(
            "ABV UI contract mismatch: expected {}, observed {}",
            abv_contract::UI_FINGERPRINT,
            presented.state.contract
        ),
    )?;
    app.wait_until(
        Duration::from_secs(30),
        "nonblack pixels on the headless Wayland output",
        || Ok(visible(&harness.testbed.capture_wayland()?)),
    )?;
    app.terminate()
}

fn water_persists(harness: &Harness<'_>) -> Result<()> {
    let app = harness.launch(true)?;
    let mut story = harness.story(&app)?;
    let initial = story.wait(water_is("dry"))?;
    demand(
        initial.state.filter == "harmless screenshot",
        "seeded ABV filter was not restored",
    )?;
    demand(
        !initial.state.text_edit_focused,
        "cold ABV witness began in an incoherent state",
    )?;

    let recess = story.anchor(abv_contract::Target::UiRecess)?;
    let (x, y) = recess.center();
    let opened = story.session().click(x, y, Button::Primary)?;
    let _open = story
        .reaction(opened)
        .until(Condition::new("UI recess open", |state: &Observation| {
            state.ui_open
        }))?;
    let dry_frame = story.capture()?;

    let wet = story.anchor(abv_contract::Target::Water(abv_contract::Water::Wet))?;
    let (x, y) = wet.center();
    let selected = story.session().click(x, y, Button::Primary)?;
    let _wet = story.reaction(selected).until(water_is("wet"))?;
    let animated = story
        .session()
        .wait_changed(&dry_frame, 0.001, 2, Duration::from_secs(5))?;
    demand(
        dry_frame.difference(&animated, 2)? > 0.001,
        "wet mode changed the witness but not ABV pixels",
    )?;
    app.wait_until(
        Duration::from_secs(5),
        "wet mode to reach slate.toml",
        || {
            Ok(harness
                .testbed
                .read_private_to_string(SLATE)
                .is_ok_and(|text| text.contains("water = \"wet\"")))
        },
    )?;
    if let Some(artifacts) = harness.artifacts {
        dry_frame.save_png(artifacts.join("abv-dry.png"))?;
        animated.save_png(artifacts.join("abv-wet.png"))?;
    }
    app.terminate()?;
    drop(story);
    drop(app);

    let restarted = harness.launch(true)?;
    let mut story = harness.story(&restarted)?;
    let _restored = story.wait(water_is("wet"))?;
    let dry = story.anchor(abv_contract::Target::Water(abv_contract::Water::Dry))?;
    let (x, y) = dry.center();
    let selected = story.session().click(x, y, Button::Primary)?;
    let _dry = story.reaction(selected).until(water_is("dry"))?;
    restarted.terminate()?;
    Ok(())
}

fn keyboard_contract(harness: &Harness<'_>) -> Result<()> {
    const WAIT: Duration = Duration::from_secs(5);

    let app = harness.launch(true)?;
    let mut focus: Probe<Observation> = app.witness()?.typed();
    let mut story = harness.story(&app)?;
    let initial = story.wait(Condition::new(
        "command guide and reference query closed",
        |state: &Observation| !state.guide_open && !state.query_open,
    ))?;
    let initial_group = initial.state.active_group.clone();

    let before = story.capture()?;
    let _opened = story.key(Key::Function(1))?.until(Condition::new(
        "command guide open",
        |state: &Observation| state.guide_open,
    ))?;
    let guide = focus.wait_anchor(&app, &abv_contract::Target::CommandGuide.to_string(), WAIT)?;
    let _presented_after_guide = focus.wait_fresh(&app, WAIT)?;
    let _compositor_margin = focus.wait_fresh(&app, WAIT)?;
    let guide_region = PixelRegion::anchor(&guide);
    let visible = story
        .session()
        .wait_changed_region(&before, guide_region, 0.55, 2, WAIT)?;
    demand(
        before.difference_region(&visible, guide_region, 2)? > 0.55,
        "F1 changed the witness without presenting the generated command guide",
    )?;
    if let Some(artifacts) = harness.artifacts {
        before.save_png(artifacts.join("abv-before-command-guide.png"))?;
        visible.save_png(artifacts.join("abv-command-guide.png"))?;
    }

    let blocked = story
        .chord(Modifiers::ALT, Key::Character('g'))?
        .next_frame()?
        .into_value();
    demand(
        blocked.state.guide_open && blocked.state.active_group == initial_group,
        "Alt+G escaped through the open command guide",
    )?;
    let _closed = story.key(Key::Escape)?.until(Condition::new(
        "command guide closed",
        |state: &Observation| !state.guide_open,
    ))?;
    let _current = focus.read()?;
    let _modal_retired = focus.wait_fresh(&app, WAIT)?;
    let _focus_settled = focus.wait_fresh(&app, WAIT)?;

    let _focused = story
        .chord(Modifiers::ALT, Key::Character('f'))?
        .until(Condition::new(
            "reference query opened",
            |state: &Observation| state.query_open,
        ))?;
    let _tag_entry = focus.wait_focus(&app, &abv_contract::Target::TagEntry.to_string(), WAIT)?;
    let deferred = story
        .chord(Modifiers::ALT, Key::Character('g'))?
        .next_frame()?
        .into_value();
    demand(
        deferred.state.active_group == initial_group,
        "query-group command stole a chord from focused text entry",
    )?;

    let _previous_control = story.chord(Modifiers::SHIFT, Key::Tab)?.next_frame()?;
    let query_panel = abv_contract::Target::Panel("reference-query").to_string();
    let _query_header = focus.wait_focus(&app, &query_panel, WAIT)?;
    let _next_control = story.key(Key::Tab)?.next_frame()?;
    let _tag_entry = focus.wait_focus(&app, &abv_contract::Target::TagEntry.to_string(), WAIT)?;

    let _next_panel = story.chord(Modifiers::CTRL, Key::Tab)?.next_frame()?;
    let gallery_panel = abv_contract::Target::Panel("gallery-controls").to_string();
    let _gallery_header = focus.wait_focus(&app, &gallery_panel, WAIT)?;
    let _previous_panel = story
        .chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab)?
        .next_frame()?;
    let _query_header = focus.wait_focus(&app, &query_panel, WAIT)?;

    let grouped = story
        .chord(Modifiers::ALT, Key::Character('g'))?
        .until(Condition::new(
            "next query group selected",
            move |state: &Observation| state.active_group != initial_group,
        ))?;
    let cycled_group = grouped.into_value().state.active_group.clone();

    let _opened = story.key(Key::Function(1))?.until(Condition::new(
        "command guide reopened",
        |state: &Observation| state.guide_open,
    ))?;
    let _closed = story.key(Key::Escape)?.until(Condition::new(
        "command guide reclosed",
        |state: &Observation| !state.guide_open,
    ))?;
    let _current = focus.read()?;
    let _modal_retired = focus.wait_fresh(&app, WAIT)?;
    let _focus_restored = focus.wait_fresh(&app, WAIT)?;
    let _query_header = focus.wait_focus(&app, &query_panel, WAIT)?;

    let _next_panel = story.chord(Modifiers::CTRL, Key::Tab)?.next_frame()?;
    let _gallery_header = focus.wait_focus(&app, &gallery_panel, WAIT)?;
    let _opened_gallery = story.key(Key::Return)?.next_frame()?;
    let rail_target = abv_contract::Target::ImagesPerRow.to_string();
    let settled = story.wait_stable(
        WAIT,
        Duration::from_millis(150),
        "images-per-row rail geometry",
        |frame| frame.anchor(&rail_target).map(|anchor| anchor.rect),
    )?;
    let rail = settled
        .anchor(&rail_target)
        .cloned()
        .ok_or_else(|| Error::Verdict {
            detail: "settled gallery omitted its images-per-row rail".to_owned(),
        })?;
    let (x, y) = rail.center();
    let clicked = story.session().click(x, y, Button::Primary)?;
    let _clicked = story.reaction(clicked).next_frame()?;
    let _rail_focus = focus.wait_focus(&app, &rail_target, WAIT)?;
    let row_count = focus.read()?.state.images_per_row;
    let expected = row_count.saturating_sub(1);
    let _adjusted = story.key(Key::Left)?.until(Condition::new(
        "focused gallery rail adjusted",
        move |state: &Observation| state.images_per_row == expected,
    ))?;
    demand(
        focus.read()?.state.active_group == cycled_group,
        "gallery adjustment disturbed the selected query group",
    )?;

    app.terminate()
}

fn native_effects(harness: &Harness<'_>) -> Result<()> {
    let app = harness.launch(true)?;
    let mut story = harness.story(&app)?;
    let _post = story.wait(Condition::new(
        "seeded reference post visible",
        |state: &Observation| state.result_posts == 1,
    ))?;

    let _focused = story
        .chord(Modifiers::ALT, Key::Character('f'))?
        .until(Condition::new(
            "tag entry focused for native clipboard story",
            |state: &Observation| state.text_edit_focused,
        ))?;
    let _typed = story.type_text("clipboard_probe")?.next_frame()?;
    let _selected = story
        .chord(Modifiers::CTRL, Key::Character('a'))?
        .next_frame()?;
    let _copied = story
        .chord(Modifiers::CTRL, Key::Character('c'))?
        .next_frame()?;
    let clipboard = clipboard_text(harness)?;
    demand(
        clipboard == "clipboard_probe",
        format!("Ctrl+C copied `{clipboard}` instead of the tag entry"),
    )?;

    let tile = story.anchor(format!("tile:{EFFECT_POST}"))?;
    let (x, y) = tile.center();
    let _opened = story.session().click(x, y, Button::Primary)?;
    let target = format!("danbooru:{EFFECT_POST}");
    let settled = story.wait_stable(
        Duration::from_secs(5),
        Duration::from_millis(150),
        "viewer title geometry",
        |frame| frame.anchor(&target).map(|anchor| anchor.rect),
    )?;
    demand(
        !settled.state.viewer_tags_open,
        "seeded viewer unexpectedly began with its tag drawer open",
    )?;
    let initial_rect = settled
        .anchor(&target)
        .map(|anchor| anchor.rect)
        .ok_or_else(|| Error::Verdict {
            detail: "initial viewer frame omitted its Danbooru link".to_owned(),
        })?;
    let _tags = story.key(Key::Character('t'))?.until(Condition::new(
        "viewer tag drawer open",
        |state: &Observation| state.viewer_tags_open,
    ))?;
    let opened = story.wait_stable(
        Duration::from_secs(5),
        Duration::from_millis(150),
        "viewer title geometry with tag drawer",
        |frame| {
            frame
                .state
                .viewer_tags_open
                .then(|| frame.anchor(&target).map(|anchor| anchor.rect))
                .flatten()
                .filter(|rect| *rect != initial_rect)
        },
    )?;
    let open_rect = opened
        .anchor(&target)
        .map(|anchor| anchor.rect)
        .ok_or_else(|| Error::Verdict {
            detail: "open tag drawer frame omitted its Danbooru link".to_owned(),
        })?;
    let _tags = story.key(Key::Character('t'))?.until(Condition::new(
        "viewer tag drawer closed",
        |state: &Observation| !state.viewer_tags_open,
    ))?;
    let settled = story.wait_stable(
        Duration::from_secs(5),
        Duration::from_millis(150),
        "viewer title geometry after closing tag drawer",
        |frame| {
            (!frame.state.viewer_tags_open)
                .then(|| frame.anchor(&target).map(|anchor| anchor.rect))
                .flatten()
                .filter(|rect| *rect != open_rect)
        },
    )?;
    let link = settled
        .anchor(&target)
        .cloned()
        .ok_or_else(|| Error::Verdict {
            detail: "settled viewer frame omitted its Danbooru link".to_owned(),
        })?;
    if let Some(artifacts) = harness.artifacts {
        story
            .capture()?
            .save_png(artifacts.join("abv-native-effects.png"))?;
    }

    let (x, y) = link.center();
    let _opened = story.session().click(x, y, Button::Primary)?;
    app.wait_until(Duration::from_secs(5), "Danbooru URL dispatch", || {
        Ok(harness.testbed.read_private(BROWSER_RECORD).is_ok())
    })?;
    let opened = harness.testbed.read_private_to_string(BROWSER_RECORD)?;
    let expected = format!("https://danbooru.donmai.us/posts/{EFFECT_POST}");
    demand(
        opened == expected,
        format!("Danbooru link dispatched `{opened}` instead of `{expected}`"),
    )?;
    app.terminate()
}

fn clipboard_text(harness: &Harness<'_>) -> Result<String> {
    let reader = harness.testbed.launch(
        AppCommand::new(harness.helper)
            .arg("--read-clipboard")
            .network(Network::Deny)
            .runtime(Duration::from_secs(5)),
    )?;
    let exit = reader.wait(Duration::from_secs(5))?;
    demand(
        exit.success(),
        format!("clipboard reader failed: {}", exit.stderr.trim()),
    )?;
    Ok(exit.stdout.trim_end().to_owned())
}

fn water_is(expected: &'static str) -> Condition<Observation> {
    Condition::new(
        format!("water mode {expected}"),
        move |state: &Observation| state.water == expected,
    )
}

fn visible(frame: &Frame) -> bool {
    let pixels = frame.rgba().chunks_exact(4);
    let total = pixels.len();
    let painted = pixels.filter(|pixel| pixel[..3] != [0, 0, 0]).count();
    painted > total / 4
}

fn seed(testbed: &Testbed) -> Result<()> {
    let mut config = DEMO_CONFIG.to_vec();
    config.extend_from_slice(b"\n[mirror]\npolicy = \"paused\"\n");
    let _config = testbed.write_private("xdg/config/adequate_booru_viewer/config.toml", &config)?;
    let _slate = testbed.write_private(SLATE, DEMO_SLATE)?;
    let data = testbed.create_private_dir("xdg/data/adequate_booru_viewer")?;
    let index = Index::open(&data.join("index.redb")).map_err(|error| Error::Verdict {
        detail: format!("create acceptance index: {error:#}"),
    })?;
    let tags = ["blue_hair", "red_eyes", "1girl", "looking_at_viewer"]
        .into_iter()
        .map(|tag| {
            Tag::forge(tag).ok_or_else(|| Error::Verdict {
                detail: format!("invalid acceptance tag `{tag}`"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    index
        .absorb(&[PostRecord {
            id: PostId(EFFECT_POST),
            rating: Rating::General,
            score: 42,
            favs: 7,
            width: 640,
            height: 480,
            created_at: "2026-08-11T00:00:00Z".to_owned(),
            tags,
            tag_hints: Vec::new(),
            preview_url: Some("https://example.test/reference.jpg".to_owned()),
            thumb_360_url: None,
            thumb_720_url: None,
            large_url: None,
            file_url: None,
        }])
        .map_err(|error| Error::Verdict {
            detail: format!("seed acceptance index: {error:#}"),
        })?;
    let _effects = testbed.create_private_dir("effects")?;
    Ok(())
}

fn print_clipboard() -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| Error::Verdict {
        detail: format!("open system clipboard: {error}"),
    })?;
    let text = clipboard.get_text().map_err(|error| Error::Verdict {
        detail: format!("read system clipboard: {error}"),
    })?;
    println!("{text}");
    Ok(())
}

fn record_browser() -> Result<()> {
    let url = env::args().nth(2).ok_or_else(|| Error::Verdict {
        detail: "browser recorder received no URL".to_owned(),
    })?;
    let path = Testbed::guest_path(BROWSER_RECORD);
    std::fs::write(&path, url).map_err(|source| Error::Io {
        operation: "record browser URL",
        path,
        source,
    })
}

fn acceptance_executable() -> Result<PathBuf> {
    env::current_exe().map_err(|source| Error::Io {
        operation: "resolve acceptance executable",
        path: PathBuf::from("<current executable>"),
        source,
    })
}

fn sibling_binary(executable: &Path) -> Result<PathBuf> {
    executable
        .parent()
        .map(|parent| parent.join("abv"))
        .ok_or_else(|| Error::Verdict {
            detail: "acceptance executable has no sibling directory".to_owned(),
        })
}

struct Cli {
    artifacts: Option<PathBuf>,
    smoke: bool,
    backend: Backend,
}

impl Cli {
    fn parse() -> Result<Self> {
        let mut args = env::args_os().skip(1);
        let mut artifacts = None;
        let mut smoke = false;
        let mut backend = Backend::X11(X11Config::default());
        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("--artifacts") => {
                    artifacts = Some(PathBuf::from(args.next().ok_or_else(|| {
                        Error::Verdict {
                            detail: "--artifacts requires a path".to_owned(),
                        }
                    })?));
                }
                Some("--smoke") => smoke = true,
                Some("--backend") => {
                    let value = args.next().ok_or_else(|| Error::Verdict {
                        detail: "--backend requires x11 or wayland".to_owned(),
                    })?;
                    backend = match value.to_str() {
                        Some("x11") => Backend::X11(X11Config::default()),
                        Some("wayland") => Backend::Wayland(WaylandConfig::default()),
                        Some(value) => {
                            return Err(Error::Verdict {
                                detail: format!(
                                    "unknown acceptance backend `{value}`; expected x11 or wayland"
                                ),
                            });
                        }
                        None => {
                            return Err(Error::Verdict {
                                detail: "acceptance backend must be valid Unicode".to_owned(),
                            });
                        }
                    };
                }
                Some(flag) => {
                    return Err(Error::Verdict {
                        detail: format!("unknown acceptance option `{flag}`"),
                    });
                }
                None => {
                    return Err(Error::Verdict {
                        detail: "acceptance options must be valid Unicode".to_owned(),
                    });
                }
            }
        }
        if matches!(backend, Backend::Wayland(_)) && !smoke {
            return Err(Error::Verdict {
                detail: "Wayland admits launch-and-capture smoke only; native stories require X11"
                    .to_owned(),
            });
        }
        Ok(Self {
            artifacts,
            smoke,
            backend,
        })
    }
}
