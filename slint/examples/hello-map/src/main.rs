//! Hello Map in Slint: a map of a few places in Zürich, which a tap names,
//! with buttons that fly there, fit them in view and turn the globe on.
//!
//! With `HELLO_MAP_SNAPSHOTS=<folder>` the app takes a short walk on its own,
//! saving the window as PNG files there, and quits.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use maplibre_slint::SlintMap;
use maplibre_slint::model::{Camera, Command, Content, Event, Inputs, Options};
use serde_json::json;
use slint::ComponentHandle;

slint::include_modules!();

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
        "hitPadding": 6
    });
    if globe {
        content["projection"] = json!({"type": "globe"});
    }
    Content::parse(&content.to_string()).expect("the content is well formed")
}

fn inputs(globe: bool) -> Inputs {
    Inputs {
        style: STYLE.to_owned(),
        camera: Camera { center: [10.45, 51.16], zoom: 4.0, bearing: 0.0, pitch: 0.0 },
        content: content(globe),
        options: Options::default(),
        events: vec![Event::Load, Event::Click, Event::Moveend],
    }
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

/// Takes the map's events: a tap names the place under it.
fn take_events(app: &App, map: &RefCell<SlintMap>) {
    let mut map = map.borrow_mut();
    for event in map.take_events() {
        if event == Event::Click {
            let click = map.record().and_then(|record| record.click.clone());
            let name = click.and_then(|click| {
                click.features.iter().find(|feature| feature.layer == "places").and_then(|feature| {
                    feature.properties.get("name").and_then(serde_json::Value::as_str).map(str::to_owned)
                })
            });
            app.set_status(name.unwrap_or_else(|| "Tap a place".to_owned()).into());
        }
    }
    if let Some(record) = map.record() {
        app.set_attribution(record.attribution.clone().into());
    }
    if let Some(error) = map.error() {
        app.set_status(error.into());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new().require_wgpu_30(maplibre_slint::wgpu_configuration()).select()?;
    let app = App::new()?;
    let walking = std::env::var_os("HELLO_MAP_SNAPSHOTS");
    let mut declared = inputs(false);
    if walking.is_some() {
        // The walk's own taps are clicks; the person's pointer moves nothing.
        declared.options = Options::parse(r#"{"interactive":false}"#).expect("the options are well formed");
    }
    let map = Rc::new(RefCell::new(SlintMap::new(declared)));
    maplibre_slint::connect!(app, map)?;

    let redraw = |app: &slint::Weak<App>| {
        if let Some(app) = app.upgrade() {
            app.window().request_redraw();
        }
    };
    app.on_zurich({
        let (map, weak) = (map.clone(), app.as_weak());
        move || {
            map.borrow_mut().command(fly_to_zurich());
            redraw(&weak);
        }
    });
    app.on_places({
        let (map, weak) = (map.clone(), app.as_weak());
        move || {
            map.borrow_mut().command(fit_places());
            redraw(&weak);
        }
    });
    app.on_toggle_globe({
        let (map, weak) = (map.clone(), app.as_weak());
        move || {
            let Some(app) = weak.upgrade() else { return };
            let globe = !app.get_globe();
            app.set_globe(globe);
            let mut map = map.borrow_mut();
            let mut inputs = map.inputs().clone();
            inputs.content = content(globe);
            map.set_inputs(inputs);
            app.window().request_redraw();
        }
    });

    let events = slint::Timer::default();
    events.start(slint::TimerMode::Repeated, Duration::from_millis(100), {
        let (map, weak) = (map.clone(), app.as_weak());
        move || {
            if let Some(app) = weak.upgrade() {
                take_events(&app, &map);
            }
        }
    });

    let _walk = walking.map(|folder| walk(&app, &map, folder.into()));
    app.run()?;
    Ok(())
}

/// One step of the walk, at a moment of it.
type Step = Box<dyn Fn(&App, &RefCell<SlintMap>)>;

/// Saves the window at a few moments of a walk: the map loaded, flown to
/// Zürich, a tap on Zürich HB, which flying there puts in the middle, and the
/// places fit in view. Then quits.
fn walk(app: &App, map: &Rc<RefCell<SlintMap>>, folder: std::path::PathBuf) -> Vec<slint::Timer> {
    let steps: Vec<(u64, Step)> = vec![
        (6000, Box::new(|app, _| snapshot(app, "1-loaded"))),
        (6100, Box::new(|_, map| map.borrow_mut().command(fly_to_zurich()))),
        (10000, Box::new(|app, _| snapshot(app, "2-zurich"))),
        (
            10100,
            Box::new(|app, map| {
                let (width, height) = (app.global::<MapLibre>().get_width(), app.global::<MapLibre>().get_height());
                map.borrow_mut().pointer(0, 0, 0, width / 2.0, height / 2.0, [false; 4]);
                map.borrow_mut().pointer(1, 0, 0, width / 2.0, height / 2.0, [false; 4]);
            }),
        ),
        (11500, Box::new(|app, _| snapshot(app, "3-tapped"))),
        (11600, Box::new(|_, map| map.borrow_mut().command(fit_places()))),
        (14000, Box::new(|app, _| snapshot(app, "4-places"))),
        (
            14100,
            Box::new(|_, _| {
                let _ = slint::quit_event_loop();
            }),
        ),
    ];
    std::fs::create_dir_all(&folder).expect("the snapshot folder can be made");
    // A window that other windows cover gets no frames from the system, so
    // the walk renders its own: a snapshot renders the window.
    let frames = slint::Timer::default();
    frames.start(slint::TimerMode::Repeated, Duration::from_millis(100), {
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                let _ = app.window().take_snapshot();
            }
        }
    });
    let snapshot_folder = Rc::new(folder);
    steps
        .into_iter()
        .map(|(at, step)| {
            let timer = slint::Timer::default();
            let (map, weak, folder) = (map.clone(), app.as_weak(), snapshot_folder.clone());
            timer.start(slint::TimerMode::SingleShot, Duration::from_millis(at), move || {
                if let Some(app) = weak.upgrade() {
                    FOLDER.with(|current| *current.borrow_mut() = Some((*folder).clone()));
                    step(&app, &map);
                    app.window().request_redraw();
                }
            });
            timer
        })
        .chain(std::iter::once(frames))
        .collect()
}

thread_local! {
    static FOLDER: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
}

fn snapshot(app: &App, name: &str) {
    let Some(folder) = FOLDER.with(|folder| folder.borrow().clone()) else { return };
    let path = folder.join(format!("{name}.png"));
    match app.window().take_snapshot() {
        Ok(pixels) => {
            let file = std::fs::File::create(&path).expect("the snapshot file can be made");
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), pixels.width(), pixels.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("the PNG header writes");
            writer.write_image_data(pixels.as_bytes()).expect("the PNG writes");
            println!("{name}: {} · {}", path.display(), app.get_status());
        }
        Err(e) => println!("{name}: no snapshot: {e}"),
    }
}
