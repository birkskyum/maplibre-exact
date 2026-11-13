//! Hello Map in Bevy: a map of a few places in Zürich, which a tap names,
//! with buttons that fly there, fit them in view and turn the globe on.
//!
//! With `HELLO_MAP_SNAPSHOTS=<folder>` the app takes a short walk on its own,
//! saving the window as PNG files there, and quits.

use std::path::PathBuf;
use std::time::Instant;

use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::picking::events::PointerClick;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::text::{FontSource, GenericFontFamily};
use bevy::window::{ExitCondition, WindowResolution};
use maplibre_bevy::model::{Camera, Command, Content, Event, Inputs, Options};
use maplibre_bevy::{
    Button as MapButton, MapCommand, MapEvent, MapLibreMap, MapLibrePlugin, MapMarker, MapPointer, Phase, Press,
};
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

/// What a button of the bar does.
#[derive(Component, Clone, Copy)]
enum Action {
    Zurich,
    Places,
    Globe,
}

#[derive(Component)]
struct Status;

#[derive(Component)]
struct Attribution;

fn main() {
    let walking = std::env::var_os("HELLO_MAP_SNAPSHOTS").map(PathBuf::from);
    let window = Window { title: "Hello Map".to_owned(), resolution: WindowResolution::new(900, 640), ..default() };
    let mut app = App::new();
    if let Some(folder) = &walking {
        // The walk draws the app into an image of its own, which it saves,
        // with no window and no event loop of the system's.
        let windows = WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() };
        app.add_plugins((
            DefaultPlugins.set(windows).disable::<bevy::winit::WinitPlugin>(),
            bevy::app::ScheduleRunnerPlugin::run_loop(std::time::Duration::from_millis(16)),
        ))
        .insert_resource(Walk::new(folder.clone()))
        .add_systems(Update, walk);
    } else {
        app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }));
    }
    app.add_plugins(MapLibrePlugin)
        .insert_resource(Walking(walking.is_some()))
        .add_systems(Startup, setup)
        .add_systems(Update, take_events)
        .run();
}

#[derive(Resource)]
struct Walking(bool);

/// The system's sans-serif font at `size`, which has the letters the default
/// font lacks, such as the ü of Zürich.
fn font(size: f32) -> TextFont {
    TextFont { font: FontSource::Generic(GenericFontFamily::SansSerif), font_size: FontSize::Px(size), ..default() }
}

fn setup(
    mut commands: Commands,
    walking: Res<Walking>,
    mut images: ResMut<Assets<Image>>,
    mut walk: Option<ResMut<Walk>>,
) {
    let camera = if let Some(walk) = walk.as_mut() {
        let mut target =
            Image::new_target_texture(1800, 1280, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
        // The walk reads the image back to save it.
        target.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
        let image = images.add(target);
        walk.image = image.clone();
        commands.spawn((Camera2d, RenderTarget::Image(ImageRenderTarget { handle: image, scale_factor: 2.0 }))).id()
    } else {
        commands.spawn(Camera2d).id()
    };
    let mut options = Options::default();
    if walking.0 {
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
    let bar = Node { padding: UiRect::all(px(8)), column_gap: px(8), align_items: AlignItems::Center, ..default() };
    commands
        .spawn((
            Node { width: percent(100), height: percent(100), flex_direction: FlexDirection::Column, ..default() },
            BackgroundColor(Color::srgb(0.1, 0.1, 0.11)),
            UiTargetCamera(camera),
        ))
        .with_children(|root| {
            root.spawn(bar.clone()).with_children(|bar| {
                bar.spawn((Text::new("Hello Map"), font(18.0), Node { flex_grow: 1.0, ..default() }));
                for (label, action) in
                    [("Zürich", Action::Zurich), ("Places", Action::Places), ("Globe", Action::Globe)]
                {
                    bar.spawn((
                        action,
                        Node { padding: UiRect::axes(px(12), px(6)), ..default() },
                        BackgroundColor(Color::srgb(0.18, 0.43, 0.93)),
                    ))
                    .with_child((Text::new(label), font(15.0), Pickable::IGNORE))
                    .observe(press);
                }
            });
            root.spawn((MapLibreMap::new(inputs), Node { flex_grow: 1.0, ..default() })).with_child((
                // A callout over Zürich HB, which the content's marker holds there.
                MapMarker { id: "station".to_owned() },
                Node {
                    padding: UiRect::axes(px(8), px(4)),
                    position_type: PositionType::Absolute,
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                children![(Text::new("Trains"), font(13.0), TextColor(Color::srgb(0.11, 0.17, 0.31)))],
            ));
            root.spawn(bar).with_children(|bar| {
                bar.spawn((Text::new("Tap a place"), font(15.0), Status, Node { flex_grow: 1.0, ..default() }));
                bar.spawn((Text::new(""), font(12.0), TextColor(Color::srgb(0.6, 0.6, 0.64)), Attribution));
            });
        });
}

fn press(
    click: On<PointerClick>,
    actions: Query<&Action>,
    mut maps: Query<(Entity, &mut MapLibreMap)>,
    mut commands: MessageWriter<MapCommand>,
    mut globe: Local<bool>,
) {
    let (Ok(action), Ok((map, mut declared))) = (actions.get(click.entity), maps.single_mut()) else { return };
    match action {
        Action::Zurich => {
            commands.write(MapCommand { map, command: fly_to_zurich() });
        }
        Action::Places => {
            commands.write(MapCommand { map, command: fit_places() });
        }
        Action::Globe => {
            *globe = !*globe;
            declared.inputs.content = content(*globe);
        }
    }
}

/// Takes the map's events: a tap names the place under it.
fn take_events(
    mut events: MessageReader<MapEvent>,
    mut status: Single<&mut Text, (With<Status>, Without<Attribution>)>,
    mut attribution: Single<&mut Text, (With<Attribution>, Without<Status>)>,
) {
    for event in events.read() {
        let Some(record) = &event.record else { continue };
        attribution.0.clone_from(&record.attribution);
        if event.event == Event::Click {
            let name = record.click.as_ref().and_then(|click| {
                click.features.iter().find(|feature| feature.layer == "places").and_then(|feature| {
                    feature.properties.get("name").and_then(serde_json::Value::as_str).map(str::to_owned)
                })
            });
            status.0 = name.unwrap_or_else(|| "Tap a place".to_owned());
        }
    }
}

/// The walk `HELLO_MAP_SNAPSHOTS` takes: the window saved at a few moments,
/// the map loaded, flown to Zürich, a tap on Zürich HB, which flying there
/// puts in the middle, and the places fit in view. Then the app quits.
#[derive(Resource)]
struct Walk {
    folder: PathBuf,
    start: Instant,
    next: usize,
    /// The image the walk draws the app into.
    image: Handle<Image>,
}

impl Walk {
    fn new(folder: PathBuf) -> Walk {
        std::fs::create_dir_all(&folder).expect("the snapshot folder can be made");
        Walk { folder, start: Instant::now(), next: 0, image: Handle::default() }
    }
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
    (12500, Step::Snapshot("3-tapped")),
    (12600, Step::Command(fit_places)),
    (15000, Step::Snapshot("4-places")),
    (16000, Step::Quit),
];

fn walk(
    mut walk: ResMut<Walk>,
    mut commands: Commands,
    map: Single<(Entity, &ComputedNode), With<MapLibreMap>>,
    status: Single<&Text, With<Status>>,
    mut map_commands: MessageWriter<MapCommand>,
    mut pointers: MessageWriter<MapPointer>,
) {
    let (entity, node) = *map;
    let elapsed = walk.start.elapsed().as_millis();
    while let Some((at, step)) = STEPS.get(walk.next)
        && u128::from(*at) <= elapsed
    {
        walk.next += 1;
        match step {
            Step::Snapshot(name) => {
                let path = walk.folder.join(format!("{name}.png"));
                println!("{name}: {} · {}", path.display(), status.0);
                commands.spawn(ReadbackOnce::texture(walk.image.clone())).observe(move |read: On<ReadbackComplete>| {
                    match save(&path, &read.data, 1800, 1280) {
                        Ok(()) => {}
                        Err(e) => println!("no snapshot: {e}"),
                    }
                });
            }
            Step::Command(command) => {
                map_commands.write(MapCommand { map: entity, command: command() });
            }
            Step::TapTheMiddle => {
                let middle = node.size() * node.inverse_scale_factor() / 2.0;
                for phase in [Phase::Down, Phase::Up] {
                    let press = Press {
                        id: 0,
                        phase,
                        x: f64::from(middle.x),
                        y: f64::from(middle.y),
                        button: MapButton::Primary,
                        touch: false,
                        modifiers: Default::default(),
                    };
                    pointers.write(MapPointer { map: entity, press });
                }
            }
            Step::Quit => std::process::exit(0),
        }
    }
}

/// Saves `data`, an image of `width` by `height` RGBA pixels whose rows wgpu
/// padded to its copy alignment, as a PNG file at `path`.
fn save(path: &std::path::Path, data: &[u8], width: u32, height: u32) -> Result<(), String> {
    let row = width as usize * 4;
    let padded = data.len() / height as usize;
    let pixels: Vec<u8> = data.chunks(padded).flat_map(|line| &line[..row]).copied().collect();
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&pixels).map_err(|e| e.to_string())
}
