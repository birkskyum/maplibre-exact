//! Hello Map in egui: a map of a few places in Zürich, which a tap names,
//! with buttons that fly there, fit them in view and turn the globe on, and a
//! callout over the station. `tests/walk.rs` takes it for a walk offscreen.

use eframe::egui;
use maplibre_egui::EguiMap;
use maplibre_egui::model::{Camera, Command, Content, Event, Inputs, Options};
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

pub struct HelloMap {
    pub map: EguiMap,
    globe: bool,
    pub status: String,
}

impl Default for HelloMap {
    fn default() -> HelloMap {
        HelloMap::new()
    }
}

impl HelloMap {
    #[must_use]
    pub fn new() -> HelloMap {
        let inputs = Inputs {
            style: STYLE.to_owned(),
            camera: Camera { center: [10.45, 51.16], zoom: 4.0, bearing: 0.0, pitch: 0.0 },
            content: content(false),
            options: Options::default(),
            events: vec![Event::Load, Event::Click, Event::Moveend],
        };
        HelloMap { map: EguiMap::new(inputs), globe: false, status: "Tap a place".to_owned() }
    }

    /// Takes the map's events: a tap names the place under it.
    fn take_events(&mut self) {
        for event in self.map.take_events() {
            if event == Event::Click {
                let click = self.map.record().and_then(|record| record.click.clone());
                let name = click.and_then(|click| {
                    click.features.iter().find(|feature| feature.layer == "places").and_then(|feature| {
                        feature.properties.get("name").and_then(serde_json::Value::as_str).map(str::to_owned)
                    })
                });
                self.status = name.unwrap_or_else(|| "Tap a place".to_owned());
            }
        }
        if let Some(error) = self.map.error() {
            self.status = error.to_owned();
        }
    }
}

impl eframe::App for HelloMap {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let Some(render_state) = frame.wgpu_render_state() else { return };
        let ctx = ui.ctx().clone();
        egui::Panel::top("bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Hello Map");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Globe").clicked() {
                        self.globe = !self.globe;
                        let mut inputs = self.map.inputs().clone();
                        inputs.content = content(self.globe);
                        self.map.set_inputs(inputs);
                    }
                    if ui.button("Places").clicked() {
                        self.map.command(fit_places());
                    }
                    if ui.button("Zürich").clicked() {
                        self.map.command(fly_to_zurich());
                    }
                });
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(self.map.record().map(|record| record.attribution.as_str()).unwrap_or_default());
                });
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            self.map.show(ui, render_state);
        });
        self.map.marker(&ctx, "station", |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| ui.label("Trains"));
        });
        self.take_events();
    }
}
