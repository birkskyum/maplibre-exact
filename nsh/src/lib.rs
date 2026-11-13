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
//! While the content has markers, the map also posts `markers` after each
//! frame that moved one, whatever `events` names: where each marker at a place
//! goes ([`MarkerPlace`]). Native Shell's native views hold no children, so the
//! app holds its own view there, absolutely positioned over the map, with
//! [`at_marker`].
//!
//! On macOS and iOS the map renders into the view's Metal layer, on a thread
//! of its own, and the view takes the mouse and the trackpad, or the
//! touches, itself.

#[cfg(target_vendor = "apple")]
mod apple;
#[cfg(target_os = "ios")]
pub mod ios;
#[cfg(target_os = "macos")]
pub mod macos;

pub use maplibre_rust_ui::model;
use nsh_core::JsonValue;
use nsh_core::prelude::*;

#[cfg(target_os = "ios")]
pub use crate::ios::register;
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

/// Where a marker of the content goes, as the map's `markers` event says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MarkerPlace {
    /// The point its anchor goes to, in points from the map's top-left corner.
    pub x: f32,
    pub y: f32,
    /// Off the map or behind the globe, or before the map drew.
    pub hidden: bool,
    /// Its anchor, as fractions of the view's width and height.
    pub anchor: [f32; 2],
    /// Degrees it turns clockwise, with the map when it turns with it.
    pub rotate: f32,
    /// How much it squashes as it lies on the pitched map, 1 upright.
    pub squash: f32,
}

impl MarkerPlace {
    /// The place of the marker `name` in a `markers` event's data.
    #[must_use]
    pub fn of(data: &JsonValue, name: &str) -> Option<MarkerPlace> {
        let place = data.get(name)?;
        let number = |key: &str, default: f32| place.get(key).and_then(JsonValue::as_f64).map_or(default, |n| n as f32);
        let anchor = |index: usize| place["anchor"].get(index).and_then(JsonValue::as_f64).unwrap_or(0.5) as f32;
        Some(MarkerPlace {
            x: number("x", 0.0),
            y: number("y", 0.0),
            hidden: place.get("hidden").and_then(JsonValue::as_bool).unwrap_or(false),
            anchor: [anchor(0), anchor(1)],
            rotate: number("rotate", 0.0),
            squash: number("squash", 1.0),
        })
    }
}

/// Holds `view` at a marker's place, upright: out of the flow of the map's
/// parent, which the map fills, with its anchor on the point, and transparent
/// while the place is hidden. Turning it is the app's, with
/// [`MarkerPlace::rotate`] about its anchor.
pub fn at_marker<S: Styled>(view: S, place: Signal<MarkerPlace>) -> S {
    view.absolute()
        .left(move || px(place.get().x))
        .top(move || px(place.get().y))
        .translate_x(move || relative(-place.get().anchor[0]))
        .translate_y(move || relative(-place.get().anchor[1]))
        .opacity(move || if place.get().hidden { 0.0_f32 } else { 1.0 })
}
