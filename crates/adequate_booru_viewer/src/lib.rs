#![expect(
    unused_crate_dependencies,
    reason = "the benchmark-facing library exposes the retrieval core; the GUI binary owns the remaining package dependencies"
)]

pub mod index;
pub mod model;
pub mod posting;
pub mod trace;
pub mod wire;
pub mod xdg;
