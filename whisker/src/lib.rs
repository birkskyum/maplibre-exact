//! MapLibre maps in [Whisker](https://whisker.rs) apps, drawn by mln-rs
//! through maplibre-rust-ui's engine into a native view of their own.
//!
//! An app places a [`Map`] and declares the map in its props, as
//! maplibre-rust-ui models it ([`model`]), in JSON: `map_style`, a style URL
//! or document; `camera`, `{"center": [lng, lat], "zoom": z, "bearing": b,
//! "pitch": p}`; `content`, its own sources and layers over the style
//! ([`model::Content`]); and `options`, camera limits and which gestures move
//! the map ([`model::Options`]). It sends commands such as `flyTo` and
//! `fitBounds` through a [`MapHandle`] it gives the map, with GL JS's
//! camera options, and hears of `load`, `moveend`, `map_click`,
//! `contextmenu` and `error` with the map's record ([`MapEvent::record`]).
//! A click is `on_map_click`, since Whisker keeps `on_click` for its own
//! gesture.
//!
//! ```ignore
//! let map = MapHandle::new();
//! render! {
//!     Map(
//!         handle: map,
//!         map_style: "https://tiles.openfreemap.org/styles/liberty",
//!         camera: r#"{"center": [8.54, 47.37], "zoom": 11}"#,
//!         on_map_click: move |event: MapEvent| println!("{}", event.record()["click"]),
//!         style: css!(flex_grow: 1.0),
//!     )
//! }
//! ```
//!
//! On iOS and macOS the map's view is Swift, in this crate's package, whose
//! layer is a `CAMetalLayer` that the map renders into on a thread of its
//! own. The view hands the map its props, size and input through the C
//! functions in [`ffi`], and runs its frames.

pub mod ffi;

use std::rc::Rc;

pub use maplibre_rust_ui::model;
use serde_json::{Value as Json, json};
use whisker::platform_module::WhiskerValue;
use whisker::prelude::*;
use whisker::runtime::view::Element;
use whisker::{ElementRef, Signal};

/// The element of the view this crate's package registers, whose events
/// are required; apps place a [`Map`].
#[doc(hidden)]
#[whisker::module_element(name = "maplibre:Map", measurement = None, commands = [("command", String)])]
pub fn native_map(
    map_style: Signal<String>,
    camera: Signal<String>,
    content: Signal<String>,
    options: Signal<String>,
    style: whisker::Style,
    on_load: MapEvent,
    on_moveend: MapEvent,
    on_map_click: MapEvent,
    on_contextmenu: MapEvent,
    on_error: MapEvent,
) {
}

/// What an app does with an event of the map.
#[derive(Clone)]
pub struct MapCallback(Rc<dyn Fn(MapEvent) + 'static>);

impl<F: Fn(MapEvent) + 'static> From<F> for MapCallback {
    fn from(f: F) -> MapCallback {
        MapCallback(Rc::new(f))
    }
}

/// Hands each event to `callback`, if the app gave one.
fn forward(callback: Option<MapCallback>) -> impl Fn(MapEvent) + 'static {
    move |event| {
        if let Some(MapCallback(callback)) = &callback {
            callback(event);
        }
    }
}

/// A map, as the app declares it in JSON.
#[component]
pub fn map(
    /// A style URL, or a style document.
    map_style: Signal<String>,
    /// `{"center": [lng, lat], "zoom": z, "bearing": b, "pitch": p}`, which
    /// moves the map when it changes.
    #[prop(default = String::new().into())]
    camera: Signal<String>,
    /// The app's sources and layers over the style ([`model::Content`]).
    #[prop(default = String::new().into())]
    content: Signal<String>,
    /// Camera limits and gestures ([`model::Options`]).
    #[prop(default = String::new().into())]
    options: Signal<String>,
    style: Option<whisker::Style>,
    /// What sends the map commands.
    handle: Option<MapHandle>,
    on_load: Option<MapCallback>,
    on_moveend: Option<MapCallback>,
    on_map_click: Option<MapCallback>,
    on_contextmenu: Option<MapCallback>,
    on_error: Option<MapCallback>,
) -> Element {
    let builder = NativeMap::builder();
    let builder = match handle {
        Some(handle) => builder.element_ref(handle.r()),
        None => builder,
    };
    builder
        .map_style(map_style)
        .camera(camera)
        .content(content)
        .options(options)
        .style(style.clone().unwrap_or_default())
        .on_load(forward(on_load.clone()))
        .on_moveend(forward(on_moveend.clone()))
        .on_map_click(forward(on_map_click.clone()))
        .on_contextmenu(forward(on_contextmenu.clone()))
        .on_error(forward(on_error.clone()))
        .build()
}

#[doc(hidden)]
pub fn __whisker_element_module_definition() -> whisker::ElementModuleDefinition {
    whisker::ElementModuleDefinition::new(env!("CARGO_PKG_NAME"), [native_map_schema::element_provider()])
}

/// An event of the map, with its record.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct MapEvent {
    #[serde(default)]
    pub detail: MapEventDetail,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct MapEventDetail {
    /// The map's record as JSON ([`model::Record`]).
    #[serde(default)]
    pub record: String,
}

impl MapEvent {
    /// The map's record: the camera at rest, whether everything in view has
    /// loaded, the last click and context menu with the content's features
    /// under them, the last failure and the style's attribution.
    #[must_use]
    pub fn record(&self) -> Json {
        serde_json::from_str(&self.detail.record).unwrap_or_default()
    }
}

/// Sends a mounted map commands, given to the map as its `handle`.
#[derive(Copy, Clone)]
pub struct MapHandle {
    r: ElementRef,
}

impl MapHandle {
    #[must_use]
    pub fn new() -> MapHandle {
        MapHandle { r: ElementRef::new() }
    }

    /// The map's element, once mounted.
    #[must_use]
    pub fn r(&self) -> ElementRef {
        self.r
    }

    /// Sends the command `name` with GL JS's options for it: `jumpTo`,
    /// `easeTo`, `flyTo`, `fitBounds` or `setStyle`.
    pub fn command(&self, name: &str, options: &Json) {
        let mut command = match options {
            Json::Object(fields) => Json::Object(fields.clone()),
            _ => json!({}),
        };
        command["command"] = Json::from(name);
        let _ = self.r.command("command", WhiskerValue::String(command.to_string()));
    }

    pub fn fly_to(&self, options: &Json) {
        self.command("flyTo", options);
    }

    pub fn ease_to(&self, options: &Json) {
        self.command("easeTo", options);
    }

    pub fn jump_to(&self, options: &Json) {
        self.command("jumpTo", options);
    }

    pub fn fit_bounds(&self, options: &Json) {
        self.command("fitBounds", options);
    }
}

impl Default for MapHandle {
    fn default() -> MapHandle {
        MapHandle::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_element_declares_its_props_events_and_command() {
        let schema = native_map_schema::schema();
        assert_eq!(schema.name, "maplibre:Map");
        assert_eq!(schema.commands.iter().map(|command| command.name.as_str()).collect::<Vec<_>>(), ["command"]);
    }

    #[test]
    fn an_event_reads_its_record() {
        let event = MapEvent { detail: MapEventDetail { record: r#"{"zoom": 4, "loaded": true}"#.to_owned() } };
        assert_eq!(event.record()["zoom"], 4);
        assert_eq!(MapEvent::default().record(), Json::Null);
    }
}
