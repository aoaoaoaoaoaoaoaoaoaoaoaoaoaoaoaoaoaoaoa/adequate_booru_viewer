#![expect(
    unused_crate_dependencies,
    reason = "the retrieval benchmark reuses the package manifest while deliberately bypassing the GUI stack"
)]

use adequate_booru_viewer::{
    date::DateRange,
    index::Index,
    model::{BoolOp, GalleryTopology, PostId, Query, QueryAtom, SearchHit, Sort, TagPolarity},
    xdg::Lair,
};
use anyhow::{Context as _, Result, bail};
use roaring::RoaringBitmap;
use std::fmt::Write as _;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

const DEFAULT_ROUNDS: usize = 31;
const DEFAULT_WARMUPS: usize = 3;
const DEFAULT_LIMIT: usize = 200;

fn no_local_favorites() -> &'static Arc<RoaringBitmap> {
    static EMPTY: LazyLock<Arc<RoaringBitmap>> = LazyLock::new(|| Arc::new(RoaringBitmap::new()));
    &EMPTY
}

fn main() -> Result<()> {
    let args = Args::parse()?;
    let lair = Lair::claim()?;
    let index_path = args.index.unwrap_or_else(|| lair.index_path());
    let index = Index::open(&index_path)?;
    let stats = index.stats()?;
    let snapshot = Snapshot {
        index_path,
        posts: stats.posts,
        pending_fact_batches: stats.pending_fact_batches,
        newest: stats.newest,
        crawl_before: stats.crawl_before,
        head: git_head(),
    };
    let cases = suite(&index, args.limit)?;
    let oracle = args
        .oracle
        .as_deref()
        .map(|path| Oracle::load_or_create(path, &index, &cases, args.limit))
        .transpose()?;

    for case in &cases {
        for _ in 0..args.warmups {
            let hit = index.search_topology(
                &case.query,
                no_local_favorites(),
                case.sort,
                DateRange::default(),
                case.topology,
                args.limit,
            )?;
            if let Some(oracle) = &oracle {
                oracle.check(case, &hit)?;
            }
        }
    }

    let mut samples = cases
        .iter()
        .map(|case| {
            (
                case.name.clone(),
                Vec::<Duration>::with_capacity(args.rounds),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for round in 0..args.rounds {
        for idx in permutation(cases.len(), round) {
            let case = &cases[idx];
            let start = Instant::now();
            let hit = index.search_topology(
                &case.query,
                no_local_favorites(),
                case.sort,
                DateRange::default(),
                case.topology,
                args.limit,
            )?;
            let elapsed = start.elapsed();
            if let Some(oracle) = &oracle {
                oracle.check(case, &hit)?;
            }
            samples
                .get_mut(&case.name)
                .context("sample bucket exists")?
                .push(elapsed);
        }
    }

    let mut medians = Vec::with_capacity(cases.len());
    println!("snapshot\t{}", snapshot);
    println!(
        "rounds\t{}\nwarmups\t{}\nlimit\t{}",
        args.rounds, args.warmups, args.limit
    );
    println!("case\tview\tsort\tcandidates\tmedian_us\tp95_us\tmin_us\tmax_us\tids");
    for case in &cases {
        let times = samples.get_mut(&case.name).context("case samples")?;
        times.sort_unstable();
        let median = percentile(times, 0.50);
        let p95 = percentile(times, 0.95);
        medians.push(median.as_secs_f64());
        let hit = index.search_topology(
            &case.query,
            no_local_favorites(),
            case.sort,
            DateRange::default(),
            case.topology,
            args.limit,
        )?;
        println!(
            "{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}",
            case.name,
            case.topology.label(),
            case.sort.label(),
            hit.candidates,
            us(median),
            us(p95),
            us(*times.first().context("nonempty samples")?),
            us(*times.last().context("nonempty samples")?),
            ids(&hit).len()
        );
    }
    println!("gmean_us\t{:.3}", us_f64(gmean(&medians)));
    Ok(())
}

#[derive(Debug)]
struct Args {
    rounds: usize,
    warmups: usize,
    limit: usize,
    oracle: Option<PathBuf>,
    index: Option<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut args = env::args().skip(1);
        let mut out = Self {
            rounds: DEFAULT_ROUNDS,
            warmups: DEFAULT_WARMUPS,
            limit: DEFAULT_LIMIT,
            oracle: None,
            index: None,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--rounds" => out.rounds = value(&mut args, "--rounds")?,
                "--warmups" => out.warmups = value(&mut args, "--warmups")?,
                "--limit" => out.limit = value(&mut args, "--limit")?,
                "--oracle" => out.oracle = Some(PathBuf::from(text(&mut args, "--oracle")?)),
                "--index" => out.index = Some(PathBuf::from(text(&mut args, "--index")?)),
                "--help" | "-h" => {
                    println!(
                        "retrieval_bench [--rounds N] [--warmups N] [--limit N] [--oracle PATH] [--index PATH]"
                    );
                    std::process::exit(0);
                }
                other => bail!("unknown argument {other}"),
            }
        }
        if out.rounds == 0 {
            bail!("--rounds must be positive");
        }
        Ok(out)
    }
}

fn value<T: std::str::FromStr>(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    text(args, flag)?
        .parse()
        .with_context(|| format!("parse {flag}"))
}

fn text(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next().with_context(|| format!("{flag} needs a value"))
}

#[derive(Clone, Debug)]
struct Case {
    name: String,
    query: Query,
    sort: Sort,
    topology: GalleryTopology,
}

impl Case {
    fn flat(name: &str, raw: &str, sort: Sort) -> Result<Self> {
        Ok(Self {
            name: name.to_owned(),
            query: flat(raw)?,
            sort,
            topology: GalleryTopology::Ungrouped,
        })
    }
}

fn suite(index: &Index, limit: usize) -> Result<Vec<Case>> {
    let mut cases = vec![
        Case::flat("common_score", "1girl", Sort::Score)?,
        Case::flat("common_newest", "solo", Sort::Newest)?,
        Case::flat(
            "rating_common_score",
            "rating:g 1girl looking_at_viewer",
            Sort::Score,
        )?,
        Case {
            name: "nested_not_color_test".to_owned(),
            query: nested_not_color_test()?,
            sort: Sort::Score,
            topology: GalleryTopology::Ungrouped,
        },
        Case {
            name: "or_bodywear_score".to_owned(),
            query: or_bodywear_score()?,
            sort: Sort::Score,
            topology: GalleryTopology::Ungrouped,
        },
        Case {
            name: "xor_hair_score".to_owned(),
            query: xor_hair_score()?,
            sort: Sort::Score,
            topology: GalleryTopology::Ungrouped,
        },
        Case::flat(
            "not_laundry_score",
            "1girl solo looking_at_viewer -comic -greyscale -multiple_girls -chibi -animated",
            Sort::Score,
        )?,
    ];
    cases.push(small_candidate(index, limit)?);
    let family_cases = cases
        .iter()
        .cloned()
        .map(|mut case| {
            case.name = format!("families_{}", case.name);
            case.topology = GalleryTopology::Grouped;
            case
        })
        .collect::<Vec<_>>();
    cases.extend(family_cases);
    Ok(cases)
}

fn flat(raw: &str) -> Result<Query> {
    let mut query = Query::default();
    for term in Query::parse_terms(raw) {
        if !query.push_atom(&[], term.atom, term.polarity) {
            bail!("push atom from {raw:?}");
        }
    }
    Ok(query)
}

fn nested_not_color_test() -> Result<Query> {
    let mut q = flat("1girl looking_at_viewer rating:g")?;
    let nope = q.push_group(&[], BoolOp::And).context("not group")?;
    push(&mut q, &nope, "blue_eyes", TagPolarity::Positive)?;
    push(&mut q, &nope, "red_hair", TagPolarity::Positive)?;
    if !q.toggle_not(&nope) {
        bail!("negate nested color group");
    }
    let yes = q.push_group(&[], BoolOp::And).context("yes group")?;
    push(&mut q, &yes, "blue_hair", TagPolarity::Positive)?;
    push(&mut q, &yes, "red_eyes", TagPolarity::Positive)?;
    Ok(q)
}

fn or_bodywear_score() -> Result<Query> {
    let mut q = flat("1girl rating:g")?;
    let group = q.push_group(&[], BoolOp::Or).context("bodywear OR")?;
    for tag in ["bikini", "nude", "swimsuit"] {
        push(&mut q, &group, tag, TagPolarity::Positive)?;
    }
    Ok(q)
}

fn xor_hair_score() -> Result<Query> {
    let mut q = flat("1girl")?;
    let group = q.push_group(&[], BoolOp::Xor).context("hair XOR")?;
    for tag in ["blue_hair", "red_hair", "white_hair"] {
        push(&mut q, &group, tag, TagPolarity::Positive)?;
    }
    Ok(q)
}

fn small_candidate(index: &Index, limit: usize) -> Result<Case> {
    for raw in [
        "from_behind weapon female_focus",
        "armored_core mecha glowing",
        "fishnets umbrella night",
        "monochrome blood weapon",
        "maid coffee_shop solo",
        "1girl red_eyes blue_hair rating:g",
    ] {
        let case = Case::flat(
            &format!("small_candidate_local_sort:{raw}"),
            raw,
            Sort::Score,
        )?;
        if index
            .search_topology(
                &case.query,
                no_local_favorites(),
                case.sort,
                DateRange::default(),
                case.topology,
                limit,
            )?
            .candidates
            > 0
        {
            return Ok(case);
        }
    }
    Case::flat(
        "small_candidate_local_sort:fallback",
        "from_behind",
        Sort::Score,
    )
}

fn push(q: &mut Query, path: &[usize], raw: &str, polarity: TagPolarity) -> Result<()> {
    let atom = QueryAtom::parse(raw).with_context(|| format!("parse atom {raw:?}"))?;
    if !q.push_atom(path, atom, polarity) {
        bail!("push atom {raw:?}");
    }
    Ok(())
}

#[derive(Debug)]
struct Snapshot {
    index_path: PathBuf,
    posts: u64,
    pending_fact_batches: u64,
    newest: Option<PostId>,
    crawl_before: Option<PostId>,
    head: String,
}

impl std::fmt::Display for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "index={} posts={} pending={} newest={} crawl_before={} head={}",
            self.index_path.display(),
            self.posts,
            self.pending_fact_batches,
            self.newest.map_or(0, |id| id.0),
            self.crawl_before.map_or(0, |id| id.0),
            self.head
        )
    }
}

#[derive(Debug)]
struct Oracle {
    cases: BTreeMap<String, OracleCase>,
}

#[derive(Debug)]
struct OracleCase {
    candidates: u64,
    ids: Vec<PostId>,
}

impl Oracle {
    fn load_or_create(path: &Path, index: &Index, cases: &[Case], limit: usize) -> Result<Self> {
        if path.exists() {
            return Self::load(path);
        }
        let mut body = String::new();
        for case in cases {
            let hit = index.search_topology(
                &case.query,
                no_local_favorites(),
                case.sort,
                DateRange::default(),
                case.topology,
                limit,
            )?;
            writeln!(
                body,
                "{}\t{}\t{}",
                case.name,
                hit.candidates,
                ids(&hit)
                    .iter()
                    .map(|id| id.0.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .context("write oracle line")?;
        }
        fs::write(path, body).with_context(|| format!("write oracle {}", path.display()))?;
        Self::load(path)
    }

    fn load(path: &Path) -> Result<Self> {
        let body =
            fs::read_to_string(path).with_context(|| format!("read oracle {}", path.display()))?;
        let mut cases = BTreeMap::new();
        for (line_no, line) in body.lines().enumerate() {
            let mut fields = line.split('\t');
            let name = fields
                .next()
                .filter(|field| !field.is_empty())
                .with_context(|| format!("oracle line {} name", line_no + 1))?;
            let candidates = fields
                .next()
                .with_context(|| format!("oracle line {} candidates", line_no + 1))?
                .parse()
                .with_context(|| format!("oracle line {} candidates", line_no + 1))?;
            let ids = fields
                .next()
                .unwrap_or("")
                .split(',')
                .filter(|id| !id.is_empty())
                .map(|id| id.parse().map(PostId).context("parse oracle id"))
                .collect::<Result<Vec<_>>>()?;
            let _old = cases.insert(name.to_owned(), OracleCase { candidates, ids });
        }
        Ok(Self { cases })
    }

    fn check(&self, case: &Case, hit: &SearchHit) -> Result<()> {
        let expected = self
            .cases
            .get(&case.name)
            .with_context(|| format!("oracle case {}", case.name))?;
        let got = ids(hit);
        if expected.candidates != hit.candidates || expected.ids != got {
            bail!(
                "oracle mismatch {}: candidates {} vs {}, ids {} vs {}",
                case.name,
                expected.candidates,
                hit.candidates,
                expected.ids.len(),
                got.len()
            );
        }
        Ok(())
    }
}

fn ids(hit: &SearchHit) -> Vec<PostId> {
    hit.posts.iter().map(|post| post.id).collect()
}

fn percentile(times: &[Duration], p: f64) -> Duration {
    let slot = ((times.len() - 1) as f64 * p).round() as usize;
    times[slot]
}

fn gmean(seconds: &[f64]) -> f64 {
    (seconds.iter().map(|x| x.ln()).sum::<f64>() / seconds.len() as f64).exp()
}

fn us(duration: Duration) -> f64 {
    us_f64(duration.as_secs_f64())
}

fn us_f64(seconds: f64) -> f64 {
    seconds * 1_000_000.0
}

fn permutation(len: usize, round: usize) -> Vec<usize> {
    let mut out = (0..len).collect::<Vec<_>>();
    for i in 0..len {
        let j = (i * 5 + round * 3 + 1) % len;
        out.swap(i, j);
    }
    out
}

fn git_head() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|head| head.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}
