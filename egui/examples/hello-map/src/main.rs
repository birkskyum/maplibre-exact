//! Hello Map in egui: a map of a few places in Zürich, which a tap names,
//! with buttons that fly there, fit them in view and turn the globe on.
//!
//! With `HELLO_MAP_SNAPSHOTS=<folder>` the app takes a short walk on its own,
//! saving the window as PNG files there, and quits.

use std::path::PathBuf;
use std::time::{Duration, Instant};

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
        "hitPadding": 6
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

struct HelloMap {
    map: EguiMap,
    globe: bool,
    status: String,
    walk: Option<Walk>,
}

impl HelloMap {
    fn new(walking: Option<PathBuf>) -> HelloMap {
        let mut options = Options::default();
        if walking.is_some() {
            // The walk's own taps are clicks; the person's pointer moves nothing.
            options = Options::parse(r#"{"interactive":false}"#).expect("the options are well formed");
        }
        let inputs = Inputs {
            style: STYLE.to_owned(),
            camera: Camera { center: [10.45, 51.16], zoom: 4.0, bearing: 0.0, pitch: 0.0 },
            content: content(false),
            options,
            events: vec![Event::Load, Event::Click, Event::Moveend],
        };
        HelloMap {
            map: EguiMap::new(inputs),
            globe: false,
            status: "Tap a place".to_owned(),
            walk: walking.map(Walk::new),
        }
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
        self.take_events();
        if let Some(mut walk) = self.walk.take() {
            walk.step(&ctx, self);
            self.walk = Some(walk);
        }
    }
}

/// The walk `HELLO_MAP_SNAPSHOTS` takes: the window saved at a few moments,
/// the map loaded, flown to Zürich, a tap on Zürich HB, which flying there
/// puts in the middle, and the places fit in view. Then the app quits.
struct Walk {
    folder: PathBuf,
    start: Instant,
    next: usize,
}

/// What the walk does at a moment of it.
enum Step {
    Snapshot(&'static str),
    Command(fn() -> Command),
    TapTheMiddle,
    Quit,
}

const STEPS: [(u64, Step); 8] = [
    (6000, Step::Snapshot("1-loaded")),
    (6100, Step::Command(fly_to_zurich)),
    (10000, Step::Snapshot("2-zurich")),
    (10100, Step::TapTheMiddle),
    (11500, Step::Snapshot("3-tapped")),
    (11600, Step::Command(fit_places)),
    (14000, Step::Snapshot("4-places")),
    (15000, Step::Quit),
];

impl Walk {
    fn new(folder: PathBuf) -> Walk {
        std::fs::create_dir_all(&folder).expect("the snapshot folder can be made");
        Walk { folder, start: Instant::now(), next: 0 }
    }

    /// Runs the steps that came due.
    fn step(&mut self, ctx: &egui::Context, app: &mut HelloMap) {
        let elapsed = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        while let Some((at, step)) = STEPS.get(self.next)
            && *at <= elapsed
        {
            self.next += 1;
            match step {
                Step::Snapshot(name) => {
                    let (path, status) = (self.folder.join(format!("{name}.png")), app.status.clone());
                    snapshot(ctx, name, path, status);
                }
                Step::Command(command) => app.map.command(command()),
                Step::TapTheMiddle => {
                    let middle = ctx.content_rect().center();
                    let press = |pressed| egui::Event::PointerButton {
                        pos: middle,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    };
                    TAPS.with(|taps| taps.borrow_mut().extend([press(true), press(false)]));
                }
                Step::Quit => std::process::exit(0),
            }
        }
    }
}

thread_local! {
    /// The walk's taps, which the next frame takes as its input.
    static TAPS: std::cell::RefCell<Vec<egui::Event>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Saves the window as the PNG file `path`, and says so with the status. On
/// macOS the window server takes it, since egui's own screenshot holds up the
/// frames that follow while it reads the pixels back.
#[cfg(target_os = "macos")]
fn snapshot(_ctx: &egui::Context, name: &str, path: PathBuf, status: String) {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;
    let window = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).windows().firstObject());
    let Some(window) = window else { return println!("{name}: no window") };
    let saved = std::process::Command::new("screencapture")
        .arg("-x")
        .arg("-o")
        .arg(format!("-l{}", window.windowNumber()))
        .arg(&path)
        .status();
    match saved {
        Ok(status_code) if status_code.success() => println!("{name}: {} · {status}", path.display()),
        Ok(status_code) => println!("{name}: no snapshot: screencapture {status_code}"),
        Err(e) => println!("{name}: no snapshot: {e}"),
    }
}

#[cfg(not(target_os = "macos"))]
fn snapshot(ctx: &egui::Context, name: &'static str, path: PathBuf, status: String) {
    ctx.request_screenshot(move |image| match save(&path, &image) {
        Ok(()) => println!("{name}: {} · {status}", path.display()),
        Err(e) => println!("{name}: no snapshot: {e}"),
    });
}

/// Saves `image` as a PNG file at `path`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn save(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let [width, height] = image.size;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(image.as_raw()).map_err(|e| e.to_string())
}

fn main() -> eframe::Result {
    let walking = std::env::var_os("HELLO_MAP_SNAPSHOTS").map(PathBuf::from);
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Hello Map").with_inner_size([900.0, 640.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    if walking.is_some() {
        background(&mut options);
    }
    eframe::run_native(
        "Hello Map",
        options,
        Box::new(move |creation| {
            if walking.is_some() {
                // An inactive app's own timers stall, so the walk's clock is a
                // thread, which wakes egui through winit's event proxy.
                let ctx = creation.egui_ctx.clone();
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(Duration::from_millis(100));
                        ctx.request_repaint();
                    }
                });
            }
            Ok(Box::new(Tapped(HelloMap::new(walking))))
        }),
    )
}

/// Hello Map, with the walk's taps handed in as input.
struct Tapped(HelloMap);

impl eframe::App for Tapped {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        TAPS.with(|taps| input.events.append(&mut taps.borrow_mut()));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.0.ui(ui, frame);
    }
}

/// Shows the walk's window on a display besides the main one, where there is
/// one, so that the windows in use there don't cover it, and without taking
/// the front of the screen.
#[cfg(target_os = "macos")]
fn background(options: &mut eframe::NativeOptions) {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    options.event_loop_builder = Some(Box::new(|builder| {
        builder.with_activation_policy(ActivationPolicy::Prohibited).with_activate_ignoring_other_apps(false);
    }));
    let Some(mtm) = MainThreadMarker::new() else { return };
    let screens = NSScreen::screens(mtm);
    if let (Some(main), Some(second)) = (screens.firstObject(), screens.iter().nth(1)) {
        // AppKit's frames rise from the main screen's bottom; winit's
        // positions fall from its top.
        let (main, second) = (main.frame(), second.frame());
        let top = main.size.height - (second.origin.y + second.size.height);
        let at = [(second.origin.x + 40.0) as f32, (top + 40.0) as f32];
        options.viewport = options.viewport.clone().with_position(at);
    }
}

#[cfg(not(target_os = "macos"))]
fn background(_options: &mut eframe::NativeOptions) {}
