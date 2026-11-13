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
    let walking = std::env::var_os("HELLO_MAP_SNAPSHOTS");
    let mut backend = slint::BackendSelector::new().require_wgpu_30(maplibre_slint::wgpu_configuration());
    if walking.is_some() {
        backend = backend.with_winit_event_loop_builder(background_event_loop());
    }
    backend.select()?;
    let app = App::new()?;
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

    if let Some(folder) = walking {
        walk(&app, &map, folder.into());
    }
    app.run()?;
    Ok(())
}

/// An event loop whose app shows its window without taking the front of the
/// screen, for the walk.
fn background_event_loop() -> slint::winit_030::EventLoopBuilder {
    use slint::winit_030::{SlintEvent, winit};
    #[allow(unused_mut)]
    let mut builder = winit::event_loop::EventLoop::<SlintEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Prohibited).with_activate_ignoring_other_apps(false);
    }
    builder
}

/// One step of the walk, at a moment of it.
type Step = fn(&App, &RefCell<SlintMap>);

/// The walk's steps, by the milliseconds into it they come at: the map
/// loaded, flown to Zürich, a tap on Zürich HB, which flying there puts in
/// the middle, and the places fit in view, each saved. Then the app quits.
const STEPS: [(u64, Step); 9] = [
    (50, |app, _| move_to_second_display(app)),
    (6000, |app, _| snapshot(app, "1-loaded")),
    (6100, |_, map| map.borrow_mut().command(fly_to_zurich())),
    (10000, |app, _| snapshot(app, "2-zurich")),
    (10100, |app, map| {
        let (width, height) = (app.global::<MapLibre>().get_width(), app.global::<MapLibre>().get_height());
        map.borrow_mut().pointer(0, 0, 0, width / 2.0, height / 2.0, [false; 4]);
        map.borrow_mut().pointer(1, 0, 0, width / 2.0, height / 2.0, [false; 4]);
    }),
    (11500, |app, _| snapshot(app, "3-tapped")),
    (11600, |_, map| map.borrow_mut().command(fit_places())),
    (14000, |app, _| snapshot(app, "4-places")),
    // The event loop's teardown runs what is left of the walk, so it ends here.
    (14100, |_, _| std::process::exit(0)),
];

/// Takes the walk, saving the window into `folder`. Its clock is a thread of
/// its own, which hands each tenth of a second to the event loop: the event
/// loop of an app in the background puts off its own timers, and a window
/// that other windows cover gets no frames from the system. Each tick renders
/// the window, which a snapshot does, takes the map's events, and runs the
/// steps that came due.
fn walk(app: &App, map: &Rc<RefCell<SlintMap>>, folder: std::path::PathBuf) {
    std::fs::create_dir_all(&folder).expect("the snapshot folder can be made");
    FOLDER.with(|current| *current.borrow_mut() = Some(folder));
    WALKED.with(|walked| *walked.borrow_mut() = Some(map.clone()));
    let app = app.as_weak();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut next = 0;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            let due = STEPS.iter().skip(next).take_while(|(at, _)| *at <= elapsed).count();
            let steps = &STEPS[next..next + due];
            next += due;
            let posted = app.upgrade_in_event_loop(move |app| {
                let Some(map) = WALKED.with(|walked| walked.borrow().clone()) else { return };
                let _ = app.window().take_snapshot();
                take_events(&app, &map);
                for (_, step) in steps {
                    step(&app, &map);
                }
                app.window().request_redraw();
            });
            if posted.is_err() {
                return;
            }
        }
    });
}

/// Moves the window to a display besides the main one, where there is one,
/// so that the windows in use there don't cover it.
fn move_to_second_display(app: &App) {
    use slint::winit_030::WinitWindowAccessor;
    app.window().with_winit_window(|window| {
        let primary = window.primary_monitor();
        if let Some(second) = window.available_monitors().find(|monitor| Some(monitor) != primary.as_ref()) {
            window.set_outer_position(second.position());
        }
    });
}

thread_local! {
    static FOLDER: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
    /// The map the walk drives.
    static WALKED: RefCell<Option<Rc<RefCell<SlintMap>>>> = const { RefCell::new(None) };
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
