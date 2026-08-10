use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use egui_tester::{
    AppCommand, Application, Button, Condition, Frame, Graphics, Network, ReactionBudget, Result,
    Story, Testbed, TestbedBuilder, WindowQuery, demand,
};
use serde::Deserialize;

const TITLE: &str = "adequate booru viewer";
const SLATE: &str = "xdg/state/adequate_booru_viewer/slate.toml";
const DEMO_CONFIG: &[u8] = include_bytes!("../../../demo/wet/config.toml");
const DEMO_SLATE: &[u8] = include_bytes!("../../../demo/wet/slate.toml");

fn main() -> Result<()> {
    let cli = Cli::parse()?;
    let binary = env::var_os("ABV_ACCEPTANCE_BINARY")
        .map(PathBuf::from)
        .map_or_else(sibling_binary, Ok)?;
    let artifacts = cli
        .artifacts
        .or_else(|| env::var_os("ABV_ACCEPTANCE_ARTIFACTS").map(PathBuf::from));
    let mut builder = TestbedBuilder::default();
    if let Some(artifacts) = &artifacts {
        builder = builder.failure_artifacts(artifacts);
    }
    builder.run(|testbed| {
        seed(testbed)?;
        let harness = Harness {
            testbed,
            binary: &binary,
            artifacts: artifacts.as_deref(),
        };
        if cli.smoke {
            smoke(&harness)
        } else {
            water_persists(&harness)
        }
    })
}

#[derive(Debug, Deserialize)]
struct Observation {
    contract: String,
    water: String,
    filter: String,
    result_posts: usize,
    text_edit_focused: bool,
    ui_open: bool,
}

type AbvStory<'app, 'bed> = Story<'app, 'bed, Observation>;

struct Harness<'a> {
    testbed: &'a Testbed,
    binary: &'a Path,
    artifacts: Option<&'a Path>,
}

impl<'a> Harness<'a> {
    fn command(&self, witnessed: bool) -> AppCommand {
        let command = AppCommand::new(self.binary)
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

fn smoke(harness: &Harness<'_>) -> Result<()> {
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

fn water_persists(harness: &Harness<'_>) -> Result<()> {
    let app = harness.launch(true)?;
    let mut story = harness.story(&app)?;
    let initial = story.wait(water_is("dry"))?;
    demand(
        initial.state.filter == "harmless screenshot",
        "seeded ABV filter was not restored",
    )?;
    demand(
        initial.state.result_posts == 0 && !initial.state.text_edit_focused,
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
    println!("abv acceptance passed under {}", harness.testbed.id());
    Ok(())
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
    Ok(())
}

fn sibling_binary() -> Result<PathBuf> {
    let executable = env::current_exe().map_err(|source| egui_tester::Error::Io {
        operation: "resolve acceptance executable",
        path: PathBuf::from("<current executable>"),
        source,
    })?;
    executable
        .parent()
        .map(|parent| parent.join("abv"))
        .ok_or_else(|| egui_tester::Error::Verdict {
            detail: "acceptance executable has no sibling directory".to_owned(),
        })
}

struct Cli {
    artifacts: Option<PathBuf>,
    smoke: bool,
}

impl Cli {
    fn parse() -> Result<Self> {
        let mut args = env::args_os().skip(1);
        let mut artifacts = None;
        let mut smoke = false;
        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("--artifacts") => {
                    artifacts = Some(PathBuf::from(args.next().ok_or_else(|| {
                        egui_tester::Error::Verdict {
                            detail: "--artifacts requires a path".to_owned(),
                        }
                    })?));
                }
                Some("--smoke") => smoke = true,
                Some(flag) => {
                    return Err(egui_tester::Error::Verdict {
                        detail: format!("unknown acceptance option `{flag}`"),
                    });
                }
                None => {
                    return Err(egui_tester::Error::Verdict {
                        detail: "acceptance options must be valid Unicode".to_owned(),
                    });
                }
            }
        }
        Ok(Self { artifacts, smoke })
    }
}
