//! Hello Map in GPUI: a map of a few places in Zürich, which a tap names,
//! with buttons that fly there, fit them in view and turn the globe on, and a
//! callout over the station. `tests/walk.rs` takes it for a walk offscreen.

use gpui::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px, rgb,
};
use maplibre_gpui::model::{Camera, Command, Content, Event, Inputs, Options};
use maplibre_gpui::{GpuiMap, at_marker};
use serde_json::json;

const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";

const PLACES: [(&str, [f64; 2]); 5] = [
    ("Grossmünster", [8.5440, 47.3701]),
    ("Fraumünster", [8.5414, 47.3697]),
    ("Zürich HB", [8.5402, 47.3779]),
    ("ETH Zürich", [8.5481, 47.3763]),
    ("Opernhaus", [8.5466, 47.3653]),
];

/// The places as circles with their names, over the base style, and the
/// globe when it is on.
fn content(globe: bool) -> Content {
    let features: Vec<_> = PLACES
        .iter()
        .map(|(name, at)| {
            json!({"type": "Feature", "properties": {"name": name}, "geometry": {"type": "Point", "coordinates": at}})
        })
        .collect();
    let mut content = json!({
        "sources": {"places": {"type": "geojson", "data": {"type": "FeatureCollection", "features": features}}},
        "layers": [
            {"id": "places", "type": "circle", "source": "places", "paint": {
                "circle-radius": 8, "circle-color": "#2f6fed", "circle-stroke-width": 2, "circle-stroke-color": "#ffffff"}},
            {"id": "place-names", "type": "symbol", "source": "places", "layout": {
                "text-field": ["get", "name"], "text-font": ["Noto Sans Bold"], "text-size": 13,
                "text-anchor": "top", "text-offset": [0, 0.9]},
             "paint": {"text-color": "#1d2b4f", "text-halo-color": "#ffffff", "text-halo-width": 1.5}}
        ],
        "hitPadding": 6,
        "markers": {"station": {"lngLat": PLACES[2].1, "anchor": "bottom", "offset": [0, -14]}}
    });
    if globe {
        content["projection"] = json!({"type": "globe"});
    }
    Content::parse(&content.to_string()).expect("the content is well formed")
}

fn command(json: &serde_json::Value) -> Command {
    Command::parse(&json.to_string()).expect("the command is well formed")
}

fn fly_to_zurich() -> Command {
    command(&json!({"command": "flyTo", "center": PLACES[2].1, "zoom": 14, "duration": 2000}))
}

fn fit_places() -> Command {
    let (lngs, lats): (Vec<f64>, Vec<f64>) = PLACES.iter().map(|(_, [lng, lat])| (*lng, *lat)).unzip();
    let min = |xs: &[f64]| xs.iter().copied().fold(f64::MAX, f64::min);
    let max = |xs: &[f64]| xs.iter().copied().fold(f64::MIN, f64::max);
    command(&json!({
        "command": "fitBounds",
        "bounds": [[min(&lngs), min(&lats)], [max(&lngs), max(&lats)]],
        "padding": 60
    }))
}

fn inputs() -> Inputs {
    Inputs {
        style: STYLE.to_owned(),
        camera: Camera { center: [10.45, 51.16], zoom: 4.0, bearing: 0.0, pitch: 0.0 },
        content: content(false),
        options: Options::default(),
        events: vec![Event::Load, Event::Click, Event::Moveend, Event::Error],
    }
}

/// The app's view: the map with the station's callout over it, and a bar of
/// buttons, the place a tap named and the map's attribution under it.
pub struct HelloMap {
    pub map: Entity<GpuiMap>,
    globe: bool,
    pub status: SharedString,
    attribution: SharedString,
    _subscriptions: [Subscription; 2],
}

impl HelloMap {
    pub fn new(cx: &mut Context<Self>) -> HelloMap {
        let map = cx.new(|_| GpuiMap::new(inputs()));
        let events = cx.subscribe(&map, |app, map, event: &Event, cx| {
            let record = map.read(cx).record();
            match event {
                Event::Click => {
                    let click = record.and_then(|record| record.click.as_ref());
                    let name = click.and_then(|click| {
                        let place = click.features.iter().find(|feature| feature.layer == "places")?;
                        place.properties.get("name").and_then(serde_json::Value::as_str).map(str::to_owned)
                    });
                    app.status = name.unwrap_or_else(|| "Tap a place".to_owned()).into();
                }
                Event::Error => {
                    app.status = record.and_then(|record| record.failure.clone()).unwrap_or_default().into();
                }
                _ => {}
            }
            if let Some(record) = map.read(cx).record() {
                app.attribution = record.attribution.clone().into();
            }
            cx.notify();
        });
        // The callout follows the station when the map moves it.
        let places = cx.observe(&map, |_, _, cx| cx.notify());
        HelloMap {
            map,
            globe: false,
            status: "Tap a place".into(),
            attribution: SharedString::default(),
            _subscriptions: [events, places],
        }
    }

    pub fn fly_to_zurich(&mut self, cx: &mut Context<Self>) {
        self.map.update(cx, |map, cx| map.command(fly_to_zurich(), cx));
    }

    pub fn fit_places(&mut self, cx: &mut Context<Self>) {
        self.map.update(cx, |map, cx| map.command(fit_places(), cx));
    }

    pub fn toggle_globe(&mut self, cx: &mut Context<Self>) {
        self.globe = !self.globe;
        let globe = self.globe;
        self.map.update(cx, |map, cx| {
            let mut inputs = map.inputs().clone();
            inputs.content = content(globe);
            map.set_inputs(inputs, cx);
        });
    }
}

/// A button of the bar under the map.
fn action(id: &'static str, title: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded(px(8.0))
        .bg(rgb(0x2f6fed))
        .text_color(rgb(0xffffff))
        .cursor_pointer()
        .child(title)
}

impl Render for HelloMap {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let callouts: Vec<_> = self
            .map
            .read(cx)
            .marker_places()
            .iter()
            .filter(|place| place.name == "station")
            .map(|place| {
                at_marker(
                    place,
                    div().px_2().py_1().rounded(px(6.0)).bg(rgb(0xffffff)).text_color(rgb(0x1d2b4f)).child("Trains"),
                )
            })
            .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x101012))
            .child(div().flex_1().relative().overflow_hidden().child(self.map.clone()).children(callouts))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .child(action("zurich", "Zürich").on_click(cx.listener(|app, _, _, cx| app.fly_to_zurich(cx))))
                    .child(action("places", "Places").on_click(cx.listener(|app, _, _, cx| app.fit_places(cx))))
                    .child(action("globe", "Globe").on_click(cx.listener(|app, _, _, cx| app.toggle_globe(cx))))
                    .child(div().text_color(rgb(0xffffff)).child(self.status.clone())),
            )
            .child(div().px_3().pb_2().text_xs().text_color(rgb(0x9a9aa2)).child(self.attribution.clone()))
    }
}
