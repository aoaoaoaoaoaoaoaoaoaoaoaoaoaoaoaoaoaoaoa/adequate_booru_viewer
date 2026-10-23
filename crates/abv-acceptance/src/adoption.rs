//! Releases through 1.3.16 rooted durable state in directories named for
//! [`LEGACY_TRIPLE`]. 1.3.17–1.3.19 renamed every root and moved nothing; no
//! error surfaced because the viewer silently began afresh in empty successors
//! beside the operator's stranded, symlinked mirror. Each legacy root must be
//! adopted by rename, never copied: the link moves, not the tens of gibibytes
//! behind it, and an empty successor yields to it.

use std::{
    fs, io,
    os::unix::fs::{MetadataExt as _, symlink},
    path::Path,
};

use adequate_booru_viewer::application_paths::LEGACY_TRIPLE;
use egui_tester::{Condition, Error, Key, Result, demand};

use super::{EFFECT_POST, FIXTURE_POSTS, Harness, Observation, SESSION_STATE, session_state};

/// Every root a legacy release wrote, with the empty scaffolding 1.3.17–1.3.19
/// created inside its successor; the data root is the one an operator moves
/// onto bulk storage behind a symlink.
const ROOTS: [(&str, Option<&str>); 4] = [
    ("xdg/config", None),
    ("xdg/data", Some("models")),
    ("xdg/cache", Some("media")),
    ("xdg/state", Some("debug")),
];
const DATA: &str = "xdg/data";
/// The symlinked data root's target, beside it so the link resolves in the guest.
const MIRROR: &str = "booru_bayonet";

/// Demote the durable layout to legacy names, the data root behind a symlink,
/// beside the empty successors 1.3.17–1.3.19 created, then demand that the
/// viewer restores its credentials, open image, index, and filters, and that
/// each root was renamed: the successor keeps the legacy inode, the link
/// itself for data.
pub(super) fn legacy_roots_adopted(harness: &Harness<'_>) -> Result<()> {
    let testbed = harness.testbed;
    let (_, _, legacy) = LEGACY_TRIPLE;
    let successor = abv_contract::product_directory();
    let mut slate = SESSION_STATE.to_vec();
    slate.extend_from_slice(
        format!("\n[viewer]\npost = {EFFECT_POST}\ngallery_anchor = {EFFECT_POST}\n").as_bytes(),
    );
    let _slate = testbed.write_private(session_state(), slate)?;
    // An empty legacy root is ignored; the cache must hold something to move.
    let _media = testbed.write_private(format!("xdg/cache/{successor}/held"), b"media")?;
    let mut inodes = Vec::with_capacity(ROOTS.len());
    for (root, scaffold) in ROOTS {
        let current = testbed.private_path(format!("{root}/{successor}"))?;
        let stranded = testbed.private_path(format!("{root}/{legacy}"))?;
        if root == DATA {
            let mirror = current.with_file_name(MIRROR);
            io_at("strand mirror", &mirror, fs::rename(&current, &mirror))?;
            io_at(
                "link legacy data root",
                &stranded,
                symlink(MIRROR, &stranded),
            )?;
        } else {
            io_at(
                "strand legacy root",
                &stranded,
                fs::rename(&current, &stranded),
            )?;
        }
        inodes.push(
            io_at(
                "inspect legacy root",
                &stranded,
                fs::symlink_metadata(&stranded),
            )?
            .ino(),
        );
        let scaffold = scaffold.map_or_else(|| current.clone(), |child| current.join(child));
        io_at(
            "scaffold empty successor",
            &scaffold,
            fs::create_dir_all(&scaffold),
        )?;
    }

    let app = harness.launch(true)?;
    // Binding the story demands that the legacy credentials armed tag pushing.
    let mut story = harness.story(&app)?;
    let _restored = story.wait(Condition::new(
        "legacy open image restored",
        |state: &Observation| {
            state
                .viewer
                .is_some_and(|viewer| viewer.post == EFFECT_POST)
        },
    ))?;
    let _gallery = story.key(Key::Escape)?.until(Condition::new(
        "legacy index and filters restored behind the image",
        |state: &Observation| state.result_posts == FIXTURE_POSTS.len(),
    ))?;
    for ((root, _), inode) in ROOTS.into_iter().zip(inodes) {
        let annexed = testbed.private_path(format!("{root}/{successor}"))?;
        let stranded = testbed.private_path(format!("{root}/{legacy}"))?;
        let renamed = fs::symlink_metadata(&annexed).is_ok_and(|entry| entry.ino() == inode);
        let vacated = fs::symlink_metadata(&stranded)
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound);
        demand(
            renamed && vacated,
            format!("{root}/{legacy} was not renamed onto {root}/{successor}"),
        )?;
    }
    app.terminate()
}

fn io_at<T>(operation: &'static str, path: &Path, result: io::Result<T>) -> Result<T> {
    result.map_err(|source| Error::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })
}
