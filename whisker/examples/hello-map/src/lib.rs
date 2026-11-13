//! Hello Map in Whisker: a map of a few places in Zürich, which a tap names,
//! with buttons that fly there, fit them in view and turn the globe on, and a
//! callout over the station.
//!
//! With `HELLO_MAP_WALK` set the app takes a short walk on its own: the map
//! loads, flies to Zürich, is tapped in the middle, where Zürich HB is then,
//! and fits the places in view.

use maplibre_whisker::{Map, MapEvent, MapHandle, MapMarker};
use serde_json::{Value as Json, json};
use whisker::css::{AlignItems, FlexDirection, JustifyContent, PositionKind};
use whisker::prelude::*;
use whisker::runtime::view::Element;

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
fn content(globe: bool) -> String {
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
    content.to_string()
}

fn fly_to_zurich() -> Json {
    json!({"center": PLACES[2].1, "zoom": 14, "duration": 2000})
}

fn fit_places() -> Json {
    let (lngs, lats): (Vec<f64>, Vec<f64>) = PLACES.iter().map(|(_, [lng, lat])| (*lng, *lat)).unzip();
    let min = |xs: &[f64]| xs.iter().copied().fold(f64::MAX, f64::min);
    let max = |xs: &[f64]| xs.iter().copied().fold(f64::MIN, f64::max);
    json!({"bounds": [[min(&lngs), min(&lats)], [max(&lngs), max(&lats)]], "padding": 60})
}

/// The name of the place a click's record has under it.
fn place(record: &Json) -> Option<String> {
    let features = record["click"]["features"].as_array()?;
    let place = features.iter().find(|feature| feature["layer"] == "places")?;
    place["properties"]["name"].as_str().map(str::to_owned)
}

#[whisker::main]
pub fn app() -> Element {
    let map = MapHandle::new();
    let globe = RwSignal::new(false);
    let status = RwSignal::new(String::from("Tap a place"));
    let attribution = RwSignal::new(String::new());
    let walking = std::env::var_os("HELLO_MAP_WALK").is_some();
    if walking {
        walk(map, status);
    }
    let page = Css::new().flex_grow(1.0).flex_direction(FlexDirection::Column).background_color(Color::hex(0x101012));
    let bar = StoredValue::new(
        Css::new()
            .flex_direction(FlexDirection::Row)
            .align_items(AlignItems::Center)
            .padding(px(12))
            .padding_bottom(px(32))
            .gap(px(8)),
    );
    let button = StoredValue::new(
        Css::new()
            .background_color(Color::hex(0x2f6fed))
            .border_radius(px(8))
            .padding(px(10))
            .align_items(AlignItems::Center)
            .justify_content(JustifyContent::Center),
    );
    let label = StoredValue::new(Css::new().color(Color::hex(0xffffff)).font_size(px(15)));
    let note = StoredValue::new(Css::new().color(Color::hex(0x9a9aa2)).font_size(px(12)));
    let callout = StoredValue::new(
        Css::new()
            .position(PositionKind::Absolute)
            .background_color(Color::hex(0xffffff))
            .border_radius(px(6))
            .padding(px(6)),
    );
    let callout_label = StoredValue::new(Css::new().color(Color::hex(0x1d2b4f)).font_size(px(13)));
    render! {
        View(style: page) {
            Map(
                handle: map,
                map_style: STYLE,
                camera: r#"{"center": [10.45, 51.16], "zoom": 4}"#,
                content: computed(move || content(globe.get())),
                // The walk's own taps are clicks; the person's touches move nothing.
                options: if walking { r#"{"interactive": false}"# } else { "" },
                on_map_click: move |event: MapEvent| {
                    status.set(place(&event.record()).unwrap_or_else(|| "Tap a place".to_owned()));
                },
                on_load: move |event: MapEvent| {
                    attribution.set(event.record()["attribution"].as_str().unwrap_or_default().to_owned());
                },
                on_error: move |event: MapEvent| {
                    status.set(event.record()["failure"].as_str().unwrap_or_default().to_owned());
                },
                style: css!(flex_grow: 1.0),
            ) {
                MapMarker(marker: "station", style: callout.get()) {
                    Text(value: "Trains", style: callout_label.get())
                }
            }
            View(style: bar.get()) {
                View(style: button.get(), on_tap: move |_| map.fly_to(&fly_to_zurich())) {
                    Text(value: "Zürich", style: label.get())
                }
                View(style: button.get(), on_tap: move |_| map.fit_bounds(&fit_places())) {
                    Text(value: "Places", style: label.get())
                }
                View(style: button.get(), on_tap: move |_| globe.set(!globe.get_untracked())) {
                    Text(value: "Globe", style: label.get())
                }
                Text(value: status, style: label.get())
            }
            Text(value: attribution, style: note.get())
        }
    }
}

/// The walk: the map loads, flies to Zürich, is tapped in the middle and
/// fits the places, with a pause after each step for a screenshot.
fn walk(map: MapHandle, status: RwSignal<String>) {
    let pause = |seconds: u64| run_blocking(move || std::thread::sleep(std::time::Duration::from_secs(seconds)));
    spawn_local(async move {
        pause(10).await;
        map.fly_to(&fly_to_zurich());
        pause(6).await;
        // The map's view hands its touches to `maplibre_whisker_pointer`, so
        // the walk taps through it too, on the app's one map, whose id is 1.
        if let Some((width, height)) = maplibre_whisker::ffi::viewport(1) {
            for phase in [0, 2] {
                maplibre_whisker::ffi::maplibre_whisker_pointer(1, 0, phase, width / 2.0, height / 2.0, 0, true, 0);
            }
        }
        pause(3).await;
        println!("tapped: {}", status.get_untracked());
        map.fit_bounds(&fit_places());
    });
}
