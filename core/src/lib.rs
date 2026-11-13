//! MapLibre maps for Rust UI frameworks.
//!
//! An app declares a map ([`model::Inputs`]): a style, a camera, its own
//! content over the style, and options. It sends the map commands and hears
//! of events, and reads the map's record, all in JSON with MapLibre GL JS's
//! names, so that the app's side looks the same whichever framework hosts the
//! map. [`Engine`] draws that map with mln-rs's runtime, into what the host
//! gives it: its own textures, which the host then draws, or a surface of a
//! native view. Each framework's host lives in a crate of its own, such as
//! `maplibre-exact`, and builds on this one. A host whose map is a native
//! view with props in JSON, as Native Shell's and Whisker's are, drives the
//! engine through a [`component::Component`].
//!
//! The mln-rs crates a host needs are re-exported, so that a host and the
//! engine agree on one version of them.

mod camera;
pub mod component;
mod content;
mod engine;
pub mod model;
pub mod placement;
pub mod texture;

pub use engine::{Engine, Phase, Pointer, Viewport};
pub use {
    mln_gestures as gestures, mln_image as image, mln_map as map, mln_runtime as runtime, mln_util as util,
    mln_webgpu as webgpu, wgpu,
};
