//! MapLibre maps in [Native Shell](https://nsh.dev) apps, drawn by mln-rs
//! through maplibre-rust-ui's engine into a native view of their own.
//!
//! The crate registers the native component `maplibre.map`, which an app
//! places with [`map`] in Rust, or with
//! `nativeComponent("maplibre.map", { props: [...] })` in TSX. The app
//! declares the map in its props, with maplibre-rust-ui's model
//! ([`model`]) in JSON:
//!
//! - `style`: a style URL, or a style document.
//! - `camera`: `{"center": [lng, lat], "zoom": z, "bearing": b, "pitch": p}`.
//! - `content`: its own sources and layers over the style
//!   ([`model::Content`]).
//! - `options`: camera limits and which gestures move the map
//!   ([`model::Options`]).
//! - `events`: the events it listens for, by name (`"load click"`), all of
//!   them when it names none.
//!
//! It sends the map commands such as `flyTo` and `fitBounds` through its
//! element's ref, `map_ref.command("flyTo", json!({"center": [8.54, 47.37]}))`,
//! with GL JS's camera options. Each event comes back as a native event of
//! its name whose data is the map's record ([`model::Record`]): the camera
//! at rest, whether everything in view has loaded, the last click and context
//! menu with the content's features under them, the last failure and the
//! style's attribution.
//!
//! On macOS the map renders into the view's Metal layer, on a thread of its
//! own, and the view takes the mouse and the trackpad itself.

#[cfg(target_os = "macos")]
pub mod macos;

pub use maplibre_rust_ui::model;
use nsh_core::prelude::*;

#[cfg(target_os = "macos")]
pub use crate::macos::register;

/// The name the crate registers its native component under.
pub const NAME: &str = "maplibre.map";

/// A map: Native Shell's native view of the component the crate registers,
/// whose props the app sets with [`NativeView::prop`] and whose events it
/// hears with [`NativeView::on_native_event`].
#[must_use]
pub fn map() -> NativeView {
    native(NAME)
}
