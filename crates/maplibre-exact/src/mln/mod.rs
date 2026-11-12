//! The map drawn by mln-rs on Exact's GPU canvas.
//!
//! Exact calls a surface on one thread, so the surface keeps its mln-rs map
//! there and runs that thread's mln-rs `RunLoop` once a frame. mln-rs renders
//! on Exact's device into a texture of its own and submits that frame itself,
//! and the surface then draws the texture into Exact's target with the encoder it
//! is given, which Exact submits after. The surface asks for frames while the
//! map loads, animates or follows a gesture, and for none once it is idle.
//!
//! The map's animations run on the host's clock, so they follow an agent's
//! `clock` as the rest of an Exact app does. Tiles load in real time.

mod blit;
mod content;
mod input;

use std::rc::Rc;

use exact_gpu::{Frame, InputEvent, PointerPhase, Surface, SurfaceError, Value, wgpu};
use mln_actor::RunLoop;
use mln_gestures::{GestureEvent, GestureRecognizer};
use mln_gfx::{BackendScope, ScopeType};
use mln_map::{AnimationOptions, CameraOptions, Map, MapEvent, MapOptions, UpdateReader};
use mln_renderer::Renderer;
use mln_renderer::query::RenderedQueryOptions;
use mln_util::chrono::{Clock, Duration, TimePoint};
use mln_util::geo::{LatLng, ScreenCoordinate, WrapMode};
use mln_util::size::Size;
use serde_json::Value as Json;

use crate::protocol::{self, Camera, CameraChange, Click, Command, Content, Event, Inputs, Record};

/// The factory a GPU module's registry names.
#[must_use]
pub fn surface() -> Box<dyn Surface> {
    Box::new(MapSurface::default())
}

thread_local! {
    /// The thread's loop, which every map on it shares, since mln-rs schedules
    /// a map's work on the loop current when the map was made.
    static RUN_LOOP: RunLoop = RunLoop::new();
}

/// How long one frame may spend on the loop's work before it draws.
const LOOP_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

/// A map on a canvas.
#[derive(Default)]
pub struct MapSurface {
    inputs: Option<Inputs>,
    engine: Option<Engine>,
    /// Commands posted before the map was made.
    pending: Vec<Command>,
    events: Vec<String>,
    published: Option<Record>,
    record: Option<Record>,
    error: Option<SurfaceError>,
    input_changed: bool,
}

/// The map and what draws it, made at the first frame, on the device that
/// frame brings.
struct Engine {
    // Dropped in this order, the renderer before its map and backend.
    renderer: Renderer,
    target: mln_webgpu::OffscreenTarget,
    map: Map,
    updates: UpdateReader,
    backend: Rc<mln_webgpu::RendererBackend>,
    blit: blit::Blit,
    gestures: GestureRecognizer,
    /// Pointers down on the map.
    pointers: u32,
    /// The generation of the last update drawn.
    drawn: u64,
    /// The arguments the map last took.
    applied: Inputs,
    /// Whether the style has loaded, which content needs.
    style_loaded: bool,
    /// The content on the style, and whether `applied.content` differs.
    held: Content,
    content_changed: bool,
    /// The camera last reported at rest.
    rest: Camera,
    idle: bool,
    loaded: bool,
    animating: bool,
    click: Option<Click>,
    error: Option<String>,
    attribution: String,
    size: Size,
    pixel_ratio: f32,
    /// The map's clock at a host millisecond.
    epoch: (TimePoint, f64),
}

impl Surface for MapSurface {
    fn arguments(&self) -> Vec<(&'static str, Value)> {
        protocol::argument_defaults()
    }

    fn bind(&mut self, inputs: &[Value], _at_ms: Option<f64>) -> Result<(), SurfaceError> {
        self.inputs = Some(Inputs::from_values(inputs).map_err(SurfaceError)?);
        Ok(())
    }

    fn render(
        &mut self,
        frame: &Frame,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> bool {
        let Some(inputs) = self.inputs.clone() else { return false };
        if self.engine.is_none() {
            match Engine::new(frame, device, queue, inputs.clone()) {
                Ok(engine) => self.engine = Some(engine),
                Err(message) => {
                    self.events.push(Event::Error.name().to_owned());
                    self.error = Some(SurfaceError(message));
                    return false;
                }
            }
        }
        let Some(engine) = self.engine.as_mut() else { return false };
        engine.apply_inputs(&inputs);
        for command in std::mem::take(&mut self.pending) {
            engine.command(command, frame.now_ms);
        }
        let mut events = Vec::new();
        let wants_frame = engine.frame(frame, &mut events);
        if let Some(texture) = engine.target.color_texture() {
            engine.blit.draw(device, encoder, target, format, &texture);
        }
        if events.contains(&Event::Error) {
            self.error = engine.error.clone().map(SurfaceError);
        }
        self.record = Some(engine.record());
        self.events.extend(events.iter().map(|event| event.name().to_owned()));
        wants_frame
    }

    fn frame_ready(&self) -> bool {
        self.engine.as_ref().is_some_and(|engine| engine.target.color_texture().is_some())
    }

    fn device_lost(&mut self) {
        self.engine = None;
    }

    fn opaque(&self) -> bool {
        true
    }

    fn wants_input(&self) -> bool {
        true
    }

    fn input(&mut self, event: &InputEvent) {
        self.input_changed = false;
        if let InputEvent::Message { text, at_ms } = event {
            match Command::parse(text) {
                Ok(command) => match self.engine.as_mut() {
                    Some(engine) => engine.command(command, *at_ms),
                    None => self.pending.push(command),
                },
                Err(message) => {
                    if let Some(engine) = self.engine.as_mut() {
                        engine.error = Some(message.clone());
                    }
                    self.events.push(Event::Error.name().to_owned());
                    self.error = Some(SurfaceError(message));
                }
            }
            self.input_changed = true;
            return;
        }
        let Some(engine) = self.engine.as_mut() else { return };
        match event {
            InputEvent::Pointer { phase: PointerPhase::Down, .. } => engine.pointers += 1,
            InputEvent::Pointer { phase: PointerPhase::Up | PointerPhase::Cancel, .. } => {
                engine.pointers = engine.pointers.saturating_sub(1);
            }
            InputEvent::Blur { .. } => engine.pointers = 0,
            _ => {}
        }
        if let Some((input, at_ms)) = input::translate(event) {
            self.input_changed = engine.gestures.handle(&mut engine.map, input, at_ms) || engine.pointers > 0;
        }
    }

    fn input_changed(&self) -> bool {
        self.input_changed
    }

    fn messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }

    fn published(&mut self) -> Option<String> {
        let record = self.record.as_ref()?;
        if self.published.as_ref() == Some(record) {
            return None;
        }
        self.published = Some(record.clone());
        Some(record.to_json())
    }

    fn agent(&mut self, _request: &str) -> Option<String> {
        self.record.as_ref().map(Record::to_json)
    }

    fn take_error(&mut self) -> Option<SurfaceError> {
        self.error.take()
    }
}

impl Engine {
    fn new(frame: &Frame, device: &wgpu::Device, queue: &wgpu::Queue, inputs: Inputs) -> Result<Engine, String> {
        RUN_LOOP.with(|_| {});
        let device = mln_webgpu::Device::from_wgpu(None, device.clone(), queue.clone(), None);
        let backend = Rc::new(mln_webgpu::RendererBackend::new(&device));
        let (width, height) = frame.pixels();
        let target = mln_webgpu::OffscreenTarget::new(backend.clone(), Size::new(width, height));
        let size = logical_size(frame);
        let mut renderer = Renderer::new(backend.clone(), frame.scale, None);
        let mut map = Map::new(MapOptions::new().with_size(size).with_pixel_ratio(frame.scale))
            .map_err(|e| format!("maplibre: {}", e.message))?;
        renderer.set_event_queue(Some(map.renderer_events()));
        let updates = map.update_reader();
        load_style(&mut map, &inputs);
        map.jump_to(&camera_options(&everything(&inputs.camera)));
        let mut engine = Engine {
            renderer,
            target,
            map,
            updates,
            backend,
            blit: blit::Blit::new(device.device()),
            gestures: GestureRecognizer::default(),
            pointers: 0,
            drawn: 0,
            rest: inputs.camera,
            applied: inputs,
            style_loaded: false,
            held: Content::default(),
            content_changed: true,
            idle: false,
            loaded: false,
            animating: false,
            click: None,
            error: None,
            attribution: String::new(),
            size,
            pixel_ratio: frame.scale,
            epoch: (Clock::now(), frame.now_ms),
        };
        // The map rounds what it was given, so its own camera is the one at rest.
        engine.rest = engine.camera();
        Ok(engine)
    }

    /// Takes the arguments that changed since the map last took them. A
    /// camera argument moves only what it names, from wherever the person
    /// left the map.
    fn apply_inputs(&mut self, inputs: &Inputs) {
        if inputs.style != self.applied.style {
            load_style(&mut self.map, inputs);
            self.style_loaded = false;
        }
        let (old, new) = (self.applied.camera, inputs.camera);
        let change = CameraChange {
            center: (new.center != old.center).then_some(new.center),
            zoom: (new.zoom != old.zoom).then_some(new.zoom),
            bearing: (new.bearing != old.bearing).then_some(new.bearing),
            pitch: (new.pitch != old.pitch).then_some(new.pitch),
        };
        if change != CameraChange::default() {
            self.map.jump_to(&camera_options(&change));
        }
        if inputs.content != self.applied.content {
            self.content_changed = true;
        }
        self.applied = inputs.clone();
    }

    /// Runs `command`, posted at `at_ms` on the host's clock, where an
    /// animation it starts begins.
    fn command(&mut self, command: Command, at_ms: f64) {
        self.map.set_frame_time(Some(self.map_time(at_ms)));
        match command {
            Command::JumpTo(change) => self.map.jump_to(&camera_options(&change)),
            Command::EaseTo(animation) => {
                self.map.ease_to(&camera_options(&animation.camera), &animation_options(animation.duration));
            }
            Command::FlyTo(animation) => {
                self.map.fly_to(&camera_options(&animation.camera), &animation_options(animation.duration));
            }
            Command::SetStyle { style } => {
                self.applied.style = style;
                load_style(&mut self.map, &self.applied);
                self.style_loaded = false;
            }
        }
        self.idle = false;
    }

    /// Runs the loop, the gestures and the map for `frame`, draws the map's
    /// latest update, and returns whether the map wants another frame.
    fn frame(&mut self, frame: &Frame, events: &mut Vec<Event>) -> bool {
        self.resize(frame);
        RUN_LOOP.with(|run_loop| {
            let start = std::time::Instant::now();
            while run_loop.run_once() && start.elapsed() < LOOP_BUDGET {}
        });
        self.map.set_frame_time(Some(self.map_time(frame.now_ms)));
        self.animating = self.gestures.frame(&mut self.map, frame.now_ms);
        for gesture in self.gestures.take_events() {
            if let GestureEvent::Click { position, .. } = gesture
                && let Ok(lat_lng) = self.map.lat_lng_for_pixel(&position, WrapMode::Wrapped)
            {
                self.click = Some(Click {
                    lng_lat: [lat_lng.longitude(), lat_lng.latitude()],
                    point: [position.x, position.y],
                    features: self.features_at(position),
                });
                events.push(Event::Click);
            }
        }
        self.map_events(events);
        self.apply_content(events);
        if self.updates.generation() != self.drawn {
            self.idle = false;
            self.draw();
            self.map_events(events);
        }
        let resting = !self.animating && !self.map.in_transition() && self.pointers == 0;
        if resting {
            let camera = self.camera();
            if camera != self.rest {
                self.rest = camera;
                events.push(Event::Moveend);
            }
        }
        !self.idle || !resting
    }

    fn map_events(&mut self, events: &mut Vec<Event>) {
        for event in self.map.drain_events() {
            match event {
                MapEvent::StyleLoaded => {
                    // A new style holds none of the content.
                    self.style_loaded = true;
                    self.held = Content::default();
                    self.content_changed = true;
                    self.attribution = self.attribution();
                }
                MapEvent::Idle => {
                    self.idle = true;
                    // A tiled source knows its attribution once its TileJSON has loaded.
                    self.attribution = self.attribution();
                    if !self.loaded {
                        self.loaded = true;
                        events.push(Event::Load);
                    }
                }
                MapEvent::LoadingStarted => self.idle = false,
                MapEvent::LoadingFailed(error) => self.fail(format!("maplibre: {error:?}"), events),
                MapEvent::RenderError(error) => self.fail(format!("maplibre: {}", error.message), events),
                _ => {}
            }
        }
    }

    fn apply_content(&mut self, events: &mut Vec<Event>) {
        if !self.style_loaded || !self.content_changed {
            return;
        }
        self.content_changed = false;
        let (held, errors) = content::apply(self.map.style_mut(), &self.held, &self.applied.content);
        self.held = held;
        if let Some(message) = errors.into_iter().reduce(|all, error| format!("{all}; {error}")) {
            self.fail(message, events);
        }
        self.idle = false;
    }

    fn fail(&mut self, message: String, events: &mut Vec<Event>) {
        self.error = Some(message);
        events.push(Event::Error);
    }

    /// The features the content's layers drew at `point`, topmost first.
    fn features_at(&self, point: ScreenCoordinate) -> Vec<protocol::Feature> {
        if self.held.layers.is_empty() {
            return Vec::new();
        }
        let pad = self.held.hit_padding;
        let geometry = if pad > 0.0 {
            let (x0, y0, x1, y1) = (point.x - pad, point.y - pad, point.x + pad, point.y + pad);
            vec![
                ScreenCoordinate::new(x0, y0),
                ScreenCoordinate::new(x1, y0),
                ScreenCoordinate::new(x1, y1),
                ScreenCoordinate::new(x0, y1),
                ScreenCoordinate::new(x0, y0),
            ]
        } else {
            vec![point]
        };
        let options = RenderedQueryOptions {
            layer_ids: Some(self.held.layers.iter().map(|layer| layer.id.clone()).collect()),
            filter: None,
        };
        self.renderer
            .query_rendered_features(&geometry, &options)
            .unwrap_or_default()
            .iter()
            .map(feature)
            .collect()
    }

    fn draw(&mut self) {
        let Some(update) = self.updates.latest() else { return };
        self.drawn = update.generation;
        let backend: Rc<dyn mln_gfx::RendererBackend> = self.backend.clone();
        let _scope = BackendScope::new(&*backend, ScopeType::Explicit);
        self.renderer.render(&update.parameters, &self.target);
    }

    fn resize(&mut self, frame: &Frame) {
        let size = logical_size(frame);
        if size != self.size && self.map.set_size(size).is_ok() {
            self.size = size;
        }
        if frame.scale != self.pixel_ratio && self.map.set_pixel_ratio(frame.scale).is_ok() {
            self.renderer.set_pixel_ratio(frame.scale);
            self.pixel_ratio = frame.scale;
        }
        let (width, height) = frame.pixels();
        self.target.resize(Size::new(width, height));
    }

    fn attribution(&self) -> String {
        let sources = self.map.style().get_sources();
        let html: Vec<String> = sources.iter().filter_map(|source| source.get_attribution()).collect();
        protocol::attribution_text(html.iter().map(String::as_str))
    }

    /// The map's clock at a host millisecond.
    fn map_time(&self, at_ms: f64) -> TimePoint {
        self.epoch.0 + Duration::from_nanos(((at_ms - self.epoch.1) * 1e6) as i64)
    }

    fn camera(&self) -> Camera {
        let options = self.map.get_camera_options(None);
        let center = options.center.unwrap_or_default();
        Camera {
            center: [center.longitude(), center.latitude()],
            zoom: options.zoom.unwrap_or_default(),
            bearing: options.bearing.unwrap_or_default(),
            pitch: options.pitch.unwrap_or_default(),
        }
    }

    fn record(&self) -> Record {
        // The camera at rest, so the record changes when something happens
        // and not every frame of a gesture.
        Record {
            camera: self.rest,
            loaded: self.loaded && self.idle,
            click: self.click.clone(),
            error: self.error.clone(),
            attribution: self.attribution.clone(),
        }
    }
}

fn feature(feature: &mln_util::feature::Feature) -> protocol::Feature {
    use ::feature::{Identifier, Value as FeatureValue};
    let properties = match mln_conversion::value_to_json(&FeatureValue::Object(feature.properties.clone())) {
        Json::Object(properties) => properties,
        _ => serde_json::Map::new(),
    };
    let id = match &feature.id {
        Identifier::Null => Json::Null,
        Identifier::Uint(id) => Json::from(*id),
        Identifier::Int(id) => Json::from(*id),
        Identifier::Double(id) => Json::from(*id),
        Identifier::String(id) => Json::from(id.clone()),
    };
    protocol::Feature {
        id,
        layer: feature.layer.clone(),
        source: feature.source.clone(),
        source_layer: feature.source_layer.clone(),
        properties,
    }
}

/// The canvas's size in whole points, at least one by one.
fn logical_size(frame: &Frame) -> Size {
    Size::new((frame.width.round() as u32).max(1), (frame.height.round() as u32).max(1))
}

fn load_style(map: &mut Map, inputs: &Inputs) {
    if inputs.style_is_document() {
        map.style_mut().load_json(&inputs.style);
    } else {
        map.style_mut().load_url(&inputs.style);
    }
}

fn everything(camera: &Camera) -> CameraChange {
    CameraChange {
        center: Some(camera.center),
        zoom: Some(camera.zoom),
        bearing: Some(camera.bearing),
        pitch: Some(camera.pitch),
    }
}

fn camera_options(change: &CameraChange) -> CameraOptions {
    CameraOptions::default()
        .with_center(change.center.and_then(|[longitude, latitude]| LatLng::new(latitude, longitude).ok()))
        .with_zoom(change.zoom)
        .with_bearing(change.bearing)
        .with_pitch(change.pitch)
}

fn animation_options(duration_ms: Option<f64>) -> AnimationOptions {
    let mut options = AnimationOptions::default();
    options.duration = duration_ms.map(|ms| Duration::from_nanos((ms * 1e6) as i64));
    options
}
