//! Hello Map in Native Shell: a map of a few places in Zürich, which a tap
//! names, with buttons that fly there, fit them in view and turn the globe on,
//! and a callout over the station.
//!
//! With `HELLO_MAP_SNAPSHOTS=<folder>` the app takes a short walk on its own,
//! saving the window as PNG files there on macOS, and quits. With
//! `HELLO_MAP_WALK` it takes the walk and says when it reaches each step.

mod walk;

use maplibre_nsh::{MarkerPlace, at_marker};
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use serde_json::{Value as Json, json};

/// The app's UI set (`nsh.json` `"rust"`), whose root `nsh` mounts.
#[native_module("hello-map/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// The map, on the hosts `nsh run` and `nsh package` build for macOS.
#[cfg(target_os = "macos")]
#[native_components(macos)]
fn components(components: &nsh_platform_macos::Components) {
    maplibre_nsh::register(components);
}

/// The map, on the hosts `nsh run` and `nsh package` build for iOS.
#[cfg(target_os = "ios")]
#[native_components(ios)]
fn components(components: &nsh_platform_ios::Components) {
    maplibre_nsh::register(components);
}

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
fn content(globe: bool) -> Json {
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
    content
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

/// A button of the bar under the map.
fn action(title: &'static str, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .on_press(press)
        .px_4()
        .py_2()
        .rounded(move || theme().radius)
        .bg(move || theme().primary)
        .active(|s| s.opacity(0.7))
        .child(text(title).text_color(move || theme().primary_foreground))
}

#[component]
pub fn App() -> impl IntoElement {
    let map = element_ref();
    let globe = signal(false);
    let status = signal(String::from("Tap a place"));
    let attribution = signal(String::new());
    let station = signal(MarkerPlace { hidden: true, ..MarkerPlace::default() });
    let folder = std::env::var_os("HELLO_MAP_SNAPSHOTS");
    let walking = folder.is_some() || std::env::var_os("HELLO_MAP_WALK").is_some();
    if walking {
        walk::walk(map, status, folder.map(Into::into));
    }
    view()
        .flex_1()
        .bg(move || theme().background)
        .child(
            view()
                .flex_1()
                .child(
                    maplibre_nsh::map()
                        .element_ref(&map)
                        .prop("style", Json::from(STYLE))
                        .prop("camera", json!({"center": [10.45, 51.16], "zoom": 4}))
                        .prop("content", move || content(globe.get()))
                        // The walk's own taps are clicks; the person's pointer moves nothing.
                        .prop("options", json!({"interactive": !walking}))
                        .prop("events", json!(["load", "click", "error"]))
                        .on_native_event(move |event: &NativeEvent| {
                            match &*event.name {
                                "markers" => {
                                    station.set(MarkerPlace::of(&event.data, "station").unwrap_or_default());
                                    return;
                                }
                                "click" => status.set(place(&event.data).unwrap_or_else(|| "Tap a place".to_owned())),
                                "error" => status.set(event.data["failure"].as_str().unwrap_or_default().to_owned()),
                                _ => {}
                            }
                            attribution.set(event.data["attribution"].as_str().unwrap_or_default().to_owned());
                        })
                        .flex_1(),
                )
                .child(at_marker(
                    view()
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .bg(move || theme().background)
                        .child(text("Trains").text_sm().text_color(move || theme().foreground)),
                    station,
                )),
        )
        .child(
            view()
                .gap_2()
                .p_3()
                .pb_safe_area_plus(px(12.0))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(action("Zürich", move || map.command("flyTo", fly_to_zurich())))
                        .child(action("Places", move || map.command("fitBounds", fit_places())))
                        .child(action("Globe", move || globe.update(|globe| *globe = !*globe)))
                        .child(text(move || status.get()).flex_1().text_color(move || theme().foreground)),
                )
                .child(text(move || attribution.get()).text_xs().text_color(move || theme().muted_foreground)),
        )
}
