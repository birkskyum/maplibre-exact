//! The map, through mln-rs's runtime, whose API mirrors
//! maplibre-native-ffi's C API one to one.
//!
//! The map lives on the runtime's thread and takes commands from the
//! surface's, which run in order there and report later through their
//! completions. A snapshot of the map's state, which the surface reads at any
//! time, places markers and clicks, and gives the gestures the camera to move.
//! The render session runs on the surface's thread, while the surface services
//! it, and renders into the surface's texture.
//!
//! The map's animations run on the host's clock, so they follow an agent's
//! `clock` as the rest of an Exact app does. Tiles load in real time.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use exact_gpu::{Frame, InputEvent, PointerPhase, wgpu};
use mln_gestures::{GestureEvent, GestureOptions, GestureRecognizer};
use mln_map::{
    AnimationOptions, BoundOptions, CameraOptions, CameraUpdate, CameraUpdateMode, MapEvent, MapMode, MapProjection,
    MapView, TransformState,
};
use mln_runtime::{
    AttachOptions, CameraFitArea, CameraFitOptions, Completion, ConsumerSync, DemandMode, Device, Driver, DriverWork,
    ErrorKind, EventMask, FrameDemand, FrameDisposition, GpuSync, MapOptions, MapSnapshot, RenderSession, RenderTarget,
    RenderedQueryGeometry, RenderedQueryOptions, Runtime, RuntimeEventKind, RuntimeOptions, SourceInfo, SyncKind,
    SyncKinds,
};
use mln_util::geo::{EdgeInsets, LatLng, LatLngBounds, ScreenCoordinate, WrapMode};
use mln_util::image::PremultipliedImage;
use mln_util::size::Size;
use serde_json::Value as Json;

use super::camera::SnapshotCamera;
use super::content::{self, Base};
use super::target::MapTexture;
use crate::protocol::{self, Camera, CameraChange, Click, Command, Content, Event, Inputs, LngLat, Options, Padding, Point, Record};

/// How long a touch stays still to be a long press, in milliseconds: iOS's
/// `UILongPressGestureRecognizer` default, and about Android's.
const LONG_PRESS_MS: f64 = 500.0;

/// The map, its render session, and what the surface knows of them.
pub(super) struct Engine {
    session: RenderSession,
    driver: DriverWork,
    map: mln_runtime::Map,
    _runtime: Runtime,
    texture: MapTexture,
    /// Exact's device, which the texture is made on.
    device: wgpu::Device,
    /// Whether the texture holds a frame of its size.
    drawn: bool,
    /// The view of the last frame drawn, which markers are placed for.
    drawn_view: Option<MapView>,
    /// The map update that the last frame showed.
    drawn_update: u64,
    demands: u64,
    /// The map's events, which the runtime's thread hands over.
    events: Arc<Mutex<Vec<MapEvent>>>,
    snapshot: MapSnapshot,
    pub(super) gestures: GestureRecognizer,
    /// Pointers down on the map.
    pub(super) pointers: u32,
    /// The pointers down that haven't moved, for long presses, and for
    /// clicks while gestures don't move the map.
    presses: Vec<Press>,
    /// Clicks the surface found itself, each a click or a context menu.
    clicks: Vec<(ScreenCoordinate, Event)>,
    /// How many of the recognizer's coming clicks end long presses, which
    /// were context menus already.
    long_pressed: u32,
    /// Clicks whose features the session is finding.
    asked: Vec<Asked>,
    /// A `fitBounds` whose camera the map is finding.
    fit: Option<Fit>,
    /// The arguments the map last took.
    pub(super) applied: Inputs,
    style: StyleState,
    /// The content on the style, and whether `applied.content` differs.
    held: Content,
    pub(super) content_changed: bool,
    /// The content's commands that haven't reported yet.
    sent: Vec<content::Sent>,
    sources: Option<Completion<Vec<SourceInfo>>>,
    /// The camera last reported at rest.
    rest: Camera,
    loaded: bool,
    animating: bool,
    click: Option<Click>,
    contextmenu: Option<Click>,
    pub(super) error: Option<String>,
    /// The last failure that refuses the canvas, until the surface reports it.
    pub(super) refusal: Option<String>,
    attribution: String,
    size: Size,
    scale: f32,
    /// The instant of a host millisecond, which the map's clock counts from.
    epoch: (Instant, f64),
}

/// Where the style is: loading, or loaded and its base JSON on its way, or
/// loaded with its base known, which content needs.
enum StyleState {
    Loading,
    Fetching(Completion<String>),
    Loaded(Base),
}

/// A pointer down on the map that hasn't moved.
struct Press {
    id: u32,
    /// Where it went down.
    at: ScreenCoordinate,
    /// When it went down, in the host's milliseconds.
    since: f64,
    /// A finger or a pen, which a long press is made with.
    touch: bool,
    /// The secondary mouse button, whose click opens a context menu.
    secondary: bool,
    /// Whether it has been held long enough to be a long press.
    long: bool,
}

/// A click whose features the session finds: first those the content's
/// layers drew there, then the expansion zoom of each cluster among them.
struct Asked {
    event: Event,
    click: Click,
    query: Option<Completion<Vec<mln_util::feature::Feature>>>,
    zooms: Vec<(usize, Completion<u8>)>,
}

struct Fit {
    camera: Completion<CameraOptions>,
    /// The map's own padding, which the camera keeps.
    kept: EdgeInsets,
    duration: Option<f64>,
}

impl Engine {
    pub(super) fn new(
        frame: &Frame,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        inputs: Inputs,
    ) -> Result<Engine, String> {
        let fail = |e: mln_runtime::Error| format!("maplibre: {}", e.message);
        let runtime = Runtime::new(&RuntimeOptions::default()).map_err(fail)?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        // The handler takes effect before the map is made, on the runtime's
        // thread, which runs commands in order.
        let _handler = runtime
            .set_event_handler(move |batch| {
                let mut sink = sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                sink.extend(batch.iter().filter_map(|event| match &event.kind {
                    RuntimeEventKind::Map(event) => Some(event.clone()),
                    _ => None,
                }));
            })
            .map_err(fail)?;
        let size = logical_size(frame);
        let options = MapOptions {
            map: mln_map::MapOptions { size, pixel_ratio: frame.scale, map_mode: MapMode::Continuous, ..Default::default() },
            event_mask: EventMask::ALL_MAP_EVENTS,
            ..Default::default()
        };
        let (map, _created) = runtime.create_map(options).map_err(fail)?;
        load_style(&map, &inputs).map_err(fail)?;
        let jump = CameraUpdate { camera: camera_options(&everything(&inputs.camera)), ..Default::default() };
        let _jumped = map.update_camera(jump).map_err(fail)?;
        let texture = MapTexture::new(device, pixels(frame));
        let shared = Device::WebGpu(mln_webgpu::Device::from_wgpu(None, device.clone(), queue.clone(), None));
        let target = RenderTarget::Borrowed { size, ring: texture.ring(), device: shared };
        let attach = AttachOptions {
            driver: Driver::CallerThread,
            producer_sync_kinds: SyncKinds::of(&[SyncKind::QueueOrder]),
            ..Default::default()
        };
        let (session, _attached) = map.attach(target, attach).map_err(fail)?;
        let driver = session.take_driver_work().map_err(fail)?;
        let snapshot = map.snapshot().map_err(fail)?;
        let mut engine = Engine {
            session,
            driver,
            map,
            _runtime: runtime,
            texture,
            device: device.clone(),
            drawn: false,
            drawn_view: None,
            drawn_update: 0,
            demands: 0,
            events,
            snapshot,
            gestures: GestureRecognizer::default(),
            pointers: 0,
            presses: Vec::new(),
            clicks: Vec::new(),
            long_pressed: 0,
            asked: Vec::new(),
            fit: None,
            rest: inputs.camera,
            applied: inputs,
            style: StyleState::Loading,
            held: Content::default(),
            content_changed: true,
            sent: Vec::new(),
            sources: None,
            loaded: false,
            animating: false,
            click: None,
            contextmenu: None,
            error: None,
            refusal: None,
            attribution: String::new(),
            size,
            scale: frame.scale,
            epoch: (Instant::now(), frame.now_ms),
        };
        engine.apply_options(&engine.applied.options.clone());
        Ok(engine)
    }

    /// The texture once it holds a frame, and the view that frame shows.
    pub(super) fn frame_texture(&self) -> Option<(&wgpu::Texture, Option<&MapView>)> {
        self.drawn.then_some((&self.texture.texture, self.drawn_view.as_ref()))
    }

    /// Takes the arguments that changed since the map last took them. A
    /// camera argument moves only what it names, from wherever the person
    /// left the map.
    pub(super) fn apply_inputs(&mut self, inputs: &Inputs) {
        if inputs.style != self.applied.style {
            self.set_style(inputs);
        }
        let (old, new) = (self.applied.camera, inputs.camera);
        let change = CameraChange {
            center: (new.center != old.center).then_some(new.center),
            zoom: (new.zoom != old.zoom).then_some(new.zoom),
            bearing: (new.bearing != old.bearing).then_some(new.bearing),
            pitch: (new.pitch != old.pitch).then_some(new.pitch),
            padding: None,
        };
        if change != CameraChange::default() {
            self.animate(CameraUpdateMode::Jump, camera_options(&change), None);
        }
        if inputs.content != self.applied.content {
            self.content_changed = true;
        }
        if inputs.options != self.applied.options {
            self.apply_options(&inputs.options);
        }
        self.applied = inputs.clone();
    }

    fn set_style(&mut self, inputs: &Inputs) {
        if let Err(e) = load_style(&self.map, inputs) {
            self.error = Some(format!("maplibre: style: {}", e.message));
        }
        self.style = StyleState::Loading;
    }

    fn apply_options(&mut self, options: &Options) {
        let [[west, south], [east, north]] = options.max_bounds.unwrap_or([[-180.0, -90.0], [180.0, 90.0]]);
        let bounds = match (LatLng::new(south, west), LatLng::new(north, east)) {
            (Ok(a), Ok(b)) => LatLngBounds::hull(a, b),
            _ => LatLngBounds::world(),
        };
        let limits = BoundOptions {
            bounds: Some(bounds),
            min_zoom: Some(options.min_zoom.unwrap_or(0.0)),
            max_zoom: Some(options.max_zoom.unwrap_or(22.0)),
            min_pitch: Some(options.min_pitch.unwrap_or(0.0)),
            max_pitch: Some(options.max_pitch.unwrap_or(60.0)),
        };
        if let Err(e) = self.map.set_bounds(limits) {
            self.error = Some(format!("maplibre: options: {}", e.message));
        }
        let mut gestures = GestureOptions::default();
        if options.drag_rotate == Some(false) {
            gestures.drag_rotate = 0.0;
            gestures.rotate_threshold = f64::INFINITY;
        }
        if options.pitch_with_rotate == Some(false) || options.touch_pitch == Some(false) {
            gestures.drag_pitch = 0.0;
        }
        if options.scroll_zoom == Some(false) {
            gestures.notch_zoom = 0.0;
            gestures.scroll_zoom_rate = 0.0;
            gestures.pinch_zoom_rate = 0.0;
        }
        self.gestures.set_options(gestures);
    }

    /// Follows the pointers that stay in place, for long presses, and, while
    /// gestures don't move the map and so report none, for clicks. Returns
    /// whether it found a click.
    pub(super) fn press(&mut self, event: &InputEvent, interactive: bool) -> bool {
        let InputEvent::Pointer { id, phase, x, y, kind, buttons, at_ms, .. } = event else { return false };
        let point = ScreenCoordinate::new(f64::from(*x), f64::from(*y));
        let tolerance = self.gestures.options().click_tolerance;
        let index = self.presses.iter().position(|press| press.id == *id);
        match (phase, index) {
            (PointerPhase::Down, _) => self.presses.push(Press {
                id: *id,
                at: point,
                since: *at_ms,
                touch: *kind != exact_gpu::PointerKind::Mouse,
                secondary: buttons & 2 != 0,
                long: false,
            }),
            (PointerPhase::Move, Some(index)) => {
                let press = &self.presses[index];
                if (press.at.x - point.x).hypot(press.at.y - point.y) > tolerance {
                    self.presses.remove(index);
                }
            }
            (PointerPhase::Up, Some(index)) => {
                let press = self.presses.remove(index);
                let still = (press.at.x - point.x).hypot(press.at.y - point.y) <= tolerance;
                if press.long {
                    self.long_pressed += u32::from(interactive);
                } else if still && !interactive {
                    self.clicks.push((point, if press.secondary { Event::Contextmenu } else { Event::Click }));
                    return true;
                }
            }
            (PointerPhase::Cancel, Some(index)) => {
                self.presses.remove(index);
            }
            _ => {}
        }
        false
    }

    /// Hands `input` at `at_ms` to the gestures, which move the map from its
    /// latest snapshot. Returns whether they did anything.
    pub(super) fn gesture(&mut self, input: mln_gestures::Input, at_ms: f64) -> bool {
        self.set_frame_time(at_ms);
        self.refresh();
        let Some((state, projection)) = view_of(&self.snapshot.view) else { return false };
        let mut camera = SnapshotCamera { map: &self.map, state: &state, projection: &projection };
        self.gestures.handle(&mut camera, input, at_ms)
    }

    /// Turns a touch held still, alone, into a context menu.
    fn long_press(&mut self, now_ms: f64) {
        if let [press] = self.presses.as_mut_slice()
            && press.touch
            && !press.long
            && now_ms - press.since >= LONG_PRESS_MS
        {
            press.long = true;
            self.clicks.push((press.at, Event::Contextmenu));
        }
    }

    /// Runs `command`, posted at `at_ms` on the host's clock, where an
    /// animation it starts begins.
    pub(super) fn command(&mut self, command: Command, at_ms: f64) {
        self.set_frame_time(at_ms);
        match command {
            Command::JumpTo(change) => self.animate(CameraUpdateMode::Jump, camera_options(&change), None),
            Command::EaseTo(animation) => {
                self.animate(CameraUpdateMode::Ease, camera_options(&animation.camera), animation.duration);
            }
            Command::FlyTo(animation) => {
                self.animate(CameraUpdateMode::Fly, camera_options(&animation.camera), animation.duration);
            }
            Command::FitBounds { bounds, padding, duration } => {
                if let Err(e) = self.fit_bounds(bounds, padding.unwrap_or_default(), duration) {
                    self.error = Some(format!("maplibre: fitBounds: {}", e.message));
                }
            }
            Command::SetStyle { style } => {
                let mut inputs = self.applied.clone();
                inputs.style = style;
                self.set_style(&inputs);
                self.applied.style = inputs.style;
            }
        }
    }

    fn animate(&mut self, mode: CameraUpdateMode, camera: CameraOptions, duration_ms: Option<f64>) {
        let update = CameraUpdate { mode, camera, animation: animation_options(duration_ms), ..Default::default() };
        if let Err(e) = self.map.update_camera(update) {
            self.error = Some(format!("maplibre: camera: {}", e.message));
        }
    }

    /// Asks the map for the camera that shows `bounds` with `padding` points
    /// to spare inside the map's own padding, as GL JS's `fitBounds` finds
    /// it. The map keeps its padding, and the center moves to make up for an
    /// uneven `padding` once the camera is known.
    fn fit_bounds(
        &mut self,
        bounds: [[f64; 2]; 2],
        padding: Padding,
        duration: Option<f64>,
    ) -> Result<(), mln_runtime::Error> {
        let [[west, south], [east, north]] = bounds;
        let kept = self.snapshot.view.camera.padding;
        let [top, left, bottom, right] = padding.edges();
        let total = EdgeInsets::new(kept.top + top, kept.left + left, kept.bottom + bottom, kept.right + right)?;
        let bounds = LatLngBounds::hull(LatLng::new(south, west)?, LatLng::new(north, east)?);
        let options = CameraFitOptions { padding: total, bearing: None, pitch: None };
        let camera = self.map.camera_for(CameraFitArea::Bounds(bounds), options)?;
        self.fit = Some(Fit { camera, kept, duration });
        Ok(())
    }

    /// Moves to the camera a `fitBounds` found, centered within the map's
    /// own padding.
    fn finish_fit(
        &mut self,
        mut camera: CameraOptions,
        kept: EdgeInsets,
        duration: Option<f64>,
    ) -> Result<(), mln_runtime::Error> {
        let mut projection = MapProjection::from_view(&self.snapshot.view)?;
        projection.set_camera(&camera);
        let [width, height] = [f64::from(self.size.width), f64::from(self.size.height)];
        let focus =
            ScreenCoordinate::new((kept.left + width - kept.right) / 2.0, (kept.top + height - kept.bottom) / 2.0);
        camera.center = Some(projection.lat_lng_for_pixel(&focus, WrapMode::Wrapped)?);
        camera.padding = Some(kept);
        let mode = if duration == Some(0.0) { CameraUpdateMode::Jump } else { CameraUpdateMode::Ease };
        self.animate(mode, camera, duration);
        Ok(())
    }

    /// Runs the gestures and the map for `frame`, services the render
    /// session, and returns whether the map wants another frame.
    pub(super) fn frame(
        &mut self,
        frame: &Frame,
        decoded: &HashMap<String, PremultipliedImage>,
        events: &mut Vec<Event>,
    ) -> bool {
        self.resize(frame);
        self.set_frame_time(frame.now_ms);
        self.refresh();
        if let Some((state, projection)) = view_of(&self.snapshot.view) {
            let mut camera = SnapshotCamera { map: &self.map, state: &state, projection: &projection };
            self.animating = self.gestures.frame(&mut camera, frame.now_ms);
        }
        self.long_press(frame.now_ms);
        let mut clicks = std::mem::take(&mut self.clicks);
        for gesture in self.gestures.take_events() {
            if let GestureEvent::Click { position, button } = gesture {
                if self.long_pressed > 0 {
                    self.long_pressed -= 1;
                } else if button == mln_gestures::Button::Secondary {
                    clicks.push((position, Event::Contextmenu));
                } else {
                    clicks.push((position, Event::Click));
                }
            }
        }
        for (position, event) in clicks {
            self.ask(position, event);
        }
        self.map_events(events);
        self.poll(events);
        self.apply_content(decoded);
        self.render();
        self.refresh();
        let resting = !self.animating && !self.snapshot.transition_in_progress && self.pointers == 0 && self.fit.is_none();
        if resting {
            let camera = self.camera();
            if camera != self.rest {
                self.rest = camera;
                events.push(Event::Moveend);
            }
        }
        if self.snapshot.fully_loaded && !self.loaded {
            self.loaded = true;
            events.push(Event::Load);
            self.ask_sources();
        }
        let waiting = self.map.applied_sequence() < self.map.sent_sequence()
            || self.snapshot.update_generation > self.drawn_update
            || !self.snapshot.fully_loaded
            || !self.sent.is_empty()
            || !self.asked.is_empty()
            || self.sources.is_some()
            || !matches!(self.style, StyleState::Loaded(_));
        waiting || !resting
    }

    /// Demands a frame of what changed, services the session, which renders
    /// it here, and gives the frame straight back: the surface's draw goes to
    /// the queue after the session's frame, and the session renders the next
    /// one only on a later service.
    fn render(&mut self) {
        self.demands += 1;
        let _ = self.session.demand_frame(FrameDemand::new(DemandMode::IfChanged, self.demands));
        if let Err(e) = self.driver.service() {
            self.refusal = Some(format!("maplibre: {}", e.message));
        }
        for result in self.session.drain_frame_results().unwrap_or_default() {
            if result.disposition == FrameDisposition::Rendered {
                self.drawn = true;
                self.drawn_view = result.view;
                self.drawn_update = result.map_update_generation;
            }
        }
        if let Ok(Some(frame)) = self.session.acquire_frame() {
            let _ = frame.release(ConsumerSync::of(GpuSync::of(SyncKind::QueueOrder)));
        }
    }

    /// Asks the session for the features the content's layers drew at
    /// `point`, topmost first.
    fn ask(&mut self, point: ScreenCoordinate, event: Event) {
        let Ok(projection) = MapProjection::from_view(&self.snapshot.view) else { return };
        let Ok(lat_lng) = projection.lat_lng_for_pixel(&point, WrapMode::Wrapped) else { return };
        let click = Click {
            lng_lat: LngLat { lng: lat_lng.longitude(), lat: lat_lng.latitude() },
            point: Point { x: point.x, y: point.y },
            features: Vec::new(),
        };
        let query = if self.held.layers.is_empty() {
            None
        } else {
            let pad = self.held.hit_padding;
            let geometry = if pad > 0.0 {
                RenderedQueryGeometry::Box {
                    min: ScreenCoordinate::new(point.x - pad, point.y - pad),
                    max: ScreenCoordinate::new(point.x + pad, point.y + pad),
                }
            } else {
                RenderedQueryGeometry::Point(point)
            };
            let options = RenderedQueryOptions {
                layer_ids: Some(self.held.layers.iter().map(|layer| layer.id.clone()).collect()),
                filter: None,
            };
            Some(
                self.session
                    .query_rendered_features(geometry, options)
                    .unwrap_or_else(|e| Completion::ready(Err(e))),
            )
        };
        self.asked.push(Asked { event, click, query, zooms: Vec::new() });
    }

    fn map_events(&mut self, events: &mut Vec<Event>) {
        let drained = std::mem::take(&mut *self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        for event in drained {
            match event {
                // A new style holds none of the content. Its base comes from
                // the map, after the commands sent before, so a style loaded
                // before a newer one was set reads the newer one, or fails
                // while that loads.
                MapEvent::StyleLoaded => match self.map.loaded_style_json() {
                    Ok(base) => self.style = StyleState::Fetching(base),
                    Err(e) => self.error = Some(format!("maplibre: style: {}", e.message)),
                },
                // A tiled source knows its attribution once its TileJSON has loaded.
                MapEvent::Idle => self.ask_sources(),
                // A style that fails to load is the network's or the server's
                // doing, so the app hears of it and the canvas isn't refused.
                MapEvent::LoadingFailed(error) => {
                    self.error = Some(format!("maplibre: {error:?}"));
                    events.push(Event::Error);
                }
                MapEvent::RenderError(error) => self.fail(format!("maplibre: {}", error.message), events),
                _ => {}
            }
        }
    }

    fn ask_sources(&mut self) {
        if self.sources.is_none() {
            self.sources = self.map.style_sources().ok();
        }
    }

    /// Takes in what the map and the session finished since the last frame.
    fn poll(&mut self, events: &mut Vec<Event>) {
        if let StyleState::Fetching(base) = &self.style
            && base.is_complete()
        {
            let StyleState::Fetching(base) = std::mem::replace(&mut self.style, StyleState::Loading) else {
                unreachable!()
            };
            if let Ok(json) = base.wait() {
                self.style = StyleState::Loaded(Base::parse(&json));
                self.held = Content::default();
                self.content_changed = true;
                self.ask_sources();
            }
        }
        let (done, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.sent).into_iter().partition(|sent| sent.completion.is_complete());
        self.sent = waiting;
        for sent in done {
            let Err(error) = sent.completion.wait() else { continue };
            sent.undo.apply(&mut self.held, &error);
            let message = format!("maplibre: {}: {}", sent.what, error.message);
            if error.kind == ErrorKind::Unsupported {
                // What the engine doesn't draw, which another engine may: the
                // app hears of it, and the canvas isn't refused.
                self.error = Some(message);
                events.push(Event::Error);
            } else {
                self.fail(message, events);
            }
        }
        if self.sources.as_ref().is_some_and(Completion::is_complete)
            && let Some(Ok(sources)) = self.sources.take().map(Completion::wait)
        {
            let html: Vec<&str> = sources.iter().filter_map(|source| source.attribution.as_deref()).collect();
            self.attribution = protocol::attribution_text(html.into_iter());
        }
        if let Some(Fit { camera, kept, duration }) = self.fit.take_if(|fit| fit.camera.is_complete())
            && let Err(e) = camera.wait().and_then(|camera| self.finish_fit(camera, kept, duration))
        {
            self.error = Some(format!("maplibre: fitBounds: {}", e.message));
        }
        self.poll_clicks(events);
    }

    fn poll_clicks(&mut self, events: &mut Vec<Event>) {
        for asked in &mut self.asked {
            if asked.query.as_ref().is_some_and(Completion::is_complete)
                && let Some(query) = asked.query.take()
            {
                let found = query.wait().unwrap_or_default();
                asked.click.features = found.iter().map(feature).collect();
                for (index, found) in found.iter().enumerate() {
                    let feature = &asked.click.features[index];
                    if feature.properties.get("cluster") != Some(&Json::Bool(true)) {
                        continue;
                    }
                    let Some(id) = feature.properties.get("cluster_id").and_then(Json::as_u64) else { continue };
                    if let Ok(zoom) = self.session.cluster_expansion_zoom(found.source.clone(), id as u32) {
                        asked.zooms.push((index, zoom));
                    }
                }
            }
        }
        let (done, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.asked)
            .into_iter()
            .partition(|asked| asked.query.is_none() && asked.zooms.iter().all(|(_, zoom)| zoom.is_complete()));
        self.asked = waiting;
        for mut asked in done {
            for (index, zoom) in asked.zooms {
                asked.click.features[index].cluster_expansion_zoom = zoom.wait().ok().map(f64::from);
            }
            match asked.event {
                Event::Contextmenu => self.contextmenu = Some(asked.click),
                _ => self.click = Some(asked.click),
            }
            events.push(asked.event);
        }
    }

    fn apply_content(&mut self, decoded: &HashMap<String, PremultipliedImage>) {
        let StyleState::Loaded(base) = &self.style else { return };
        if !self.content_changed {
            return;
        }
        self.content_changed = false;
        let (held, sent) = content::apply(&self.map, base, &self.held, &self.applied.content, decoded);
        self.held = held;
        self.sent.extend(sent);
    }

    /// Reports a failure that is the app's or the engine's, which also refuses
    /// the canvas so that Exact reports it.
    fn fail(&mut self, message: String, events: &mut Vec<Event>) {
        self.error = Some(message.clone());
        self.refusal = Some(message);
        events.push(Event::Error);
    }

    /// Gives the session a texture of the canvas's new size, which resizes
    /// the map too.
    fn resize(&mut self, frame: &Frame) {
        let size = logical_size(frame);
        let pixels = pixels(frame);
        if size == self.size && pixels == self.texture.pixels() && frame.scale == self.scale {
            return;
        }
        let texture = MapTexture::new(&self.device, pixels);
        match self.session.set_ring(size, texture.ring()) {
            Ok(_) => {
                self.texture = texture;
                self.drawn = false;
                self.size = size;
                self.scale = frame.scale;
            }
            Err(e) => self.refusal = Some(format!("maplibre: {}", e.message)),
        }
    }

    fn refresh(&mut self) {
        if let Ok(snapshot) = self.map.snapshot() {
            self.snapshot = snapshot;
        }
    }

    /// Has the map's animations sample the host's clock at `at_ms`.
    fn set_frame_time(&self, at_ms: f64) {
        let at = self.map_time(at_ms);
        let _ = self.map.with_core(move |core| core.set_frame_time(Some(at)));
    }

    /// The instant of a host millisecond.
    fn map_time(&self, at_ms: f64) -> Instant {
        let since = at_ms - self.epoch.1;
        let shift = Duration::from_secs_f64(since.abs() / 1000.0);
        if since >= 0.0 { self.epoch.0 + shift } else { self.epoch.0.checked_sub(shift).unwrap_or(self.epoch.0) }
    }

    fn camera(&self) -> Camera {
        let Some((state, _)) = view_of(&self.snapshot.view) else { return self.rest };
        let options = state.get_camera_options(None);
        let center = options.center.unwrap_or_default();
        Camera {
            center: [center.longitude(), center.latitude()],
            zoom: options.zoom.unwrap_or_default(),
            bearing: options.bearing.unwrap_or_default(),
            pitch: options.pitch.unwrap_or_default(),
        }
    }

    pub(super) fn record(&self) -> Record {
        // The camera at rest, so the record changes when something happens
        // and not every frame of a gesture.
        Record {
            camera: self.rest,
            loaded: self.loaded && self.snapshot.fully_loaded,
            click: self.click.clone(),
            contextmenu: self.contextmenu.clone(),
            failure: self.error.clone(),
            attribution: self.attribution.clone(),
        }
    }
}

impl Drop for Engine {
    /// Stops the session here, since it renders only while this thread
    /// services it, and lets go of the map.
    fn drop(&mut self) {
        let _ = self.session.abandon();
        let _ = self.session.release();
        let _ = self.map.release();
    }
}

/// The transform state and projection of `view`.
fn view_of(view: &MapView) -> Option<(TransformState, MapProjection)> {
    Some((view.to_state().ok()?, MapProjection::from_view(view).ok()?))
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
        cluster_expansion_zoom: None,
    }
}

/// The canvas's size in whole points, at least one by one.
fn logical_size(frame: &Frame) -> Size {
    Size::new((frame.width.round() as u32).max(1), (frame.height.round() as u32).max(1))
}

/// The canvas's size in pixels, at least one by one.
fn pixels(frame: &Frame) -> Size {
    let (width, height) = frame.pixels();
    Size::new(width.max(1), height.max(1))
}

fn load_style(map: &mln_runtime::Map, inputs: &Inputs) -> Result<(), mln_runtime::Error> {
    let loading = if inputs.style_is_document() {
        map.set_style_json(inputs.style.clone())?
    } else {
        map.set_style_url(inputs.style.clone())?
    };
    // The style's load reports through the map's events.
    drop(loading);
    Ok(())
}

fn everything(camera: &Camera) -> CameraChange {
    CameraChange {
        center: Some(camera.center),
        zoom: Some(camera.zoom),
        bearing: Some(camera.bearing),
        pitch: Some(camera.pitch),
        padding: None,
    }
}

fn camera_options(change: &CameraChange) -> CameraOptions {
    CameraOptions {
        center: change.center.and_then(|[longitude, latitude]| LatLng::new(latitude, longitude).ok()),
        zoom: change.zoom,
        bearing: change.bearing,
        pitch: change.pitch,
        padding: change.padding.and_then(|padding| {
            let [top, left, bottom, right] = padding.edges();
            EdgeInsets::new(top, left, bottom, right).ok()
        }),
        ..CameraOptions::default()
    }
}

fn animation_options(duration_ms: Option<f64>) -> AnimationOptions {
    AnimationOptions {
        duration: duration_ms.map(|ms| Duration::from_secs_f64(ms.max(0.0) / 1000.0)),
        ..AnimationOptions::default()
    }
}
