//! A map as a component of a UI: the map the app declares, the engine once
//! the map has a size, and the input and frames the host hands it. Every host
//! but Exact's drives its maps through one. Hosts whose views are the
//! platform's own, as in Native Shell and Whisker, take the declaration as
//! JSON props and give each frame a target of the view, such as its Metal
//! layer. Hosts that draw with wgpu, as Slint and egui do, take it as
//! [`Inputs`] and give each frame their device, and the map draws into a
//! texture of its own on it. Either way the host posts the app the events each
//! frame returns.

use std::collections::HashMap;
use std::time::Instant;

use mln_gestures::{Button, Input, Modifiers, PointerKind};
use mln_runtime::{AcquiredFrame, Device, Driver, ExportKind, ExportKinds, ExportedRing, RenderTarget};
use mln_util::geo::ScreenCoordinate;
use mln_util::image::PremultipliedImage;
use mln_util::size::Size;
use serde_json::Value as Json;

use crate::model::{Camera, Command, Content, Event, Inputs, Marker, Options, Record};
use crate::placement::{self, Placement, View};
use crate::texture::MapTexture;
use crate::{Engine, Phase, Pointer, Viewport};

/// The events a map posts when its props name none.
const ALL_EVENTS: [Event; 5] = [Event::Load, Event::Moveend, Event::Click, Event::Contextmenu, Event::Error];

/// Where the camera is before the props set one: the whole world.
const WORLD: Camera = Camera { center: [0.0, 0.0], zoom: 0.0, bearing: 0.0, pitch: 0.0 };

/// What a map posts to its app: an event, by its name too, with the map's
/// record as JSON.
pub struct Post {
    pub event: Event,
    pub name: &'static str,
    pub data: Json,
}

/// What a frame did on the host's device: whether the map wants another
/// frame, what to post to the app, and the texture the map draws into when
/// it is a new one, which the host shows from then on.
/// Where a marker at a place puts its anchor, for a host whose app holds its
/// own view there rather than the map holding it.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerPlace {
    /// The marker's name in the content.
    pub name: String,
    /// The point its anchor goes to, in points from the map's top-left corner.
    pub x: f32,
    pub y: f32,
    /// Off the map or behind the globe, or before the map drew.
    pub hidden: bool,
    /// Its anchor, as fractions of the view's width and height.
    pub anchor: [f32; 2],
    /// Degrees it turns clockwise, with the map when it turns with it.
    pub rotate: f32,
    /// How much it squashes as it lies on the pitched map, 1 upright.
    pub squash: f32,
}

/// What a frame of a map that renders into images it shares gives the host.
pub struct SharedFrame {
    /// Whether the map wants another frame.
    pub again: bool,
    pub posts: Vec<Post>,
    /// The newest image the map finished since the last frame, which the host
    /// draws and drops once its own GPU work on it completed.
    pub image: Option<AcquiredFrame>,
}

pub struct TextureFrame {
    pub again: bool,
    pub posts: Vec<Post>,
    pub texture: Option<wgpu::Texture>,
}

/// A pointer as the view reports it, in logical pixels from its top-left
/// corner.
#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub id: u32,
    pub phase: Phase,
    pub x: f64,
    pub y: f64,
    pub button: Button,
    pub touch: bool,
    pub modifiers: Modifiers,
}

/// A map whose props, commands and input come from a view of the platform's
/// own. Its props are `style`, `camera`, `content`, `options` and `events`.
pub struct Component {
    inputs: Inputs,
    engine: Option<Engine>,
    /// Commands sent before the map was made.
    pending: Vec<Command>,
    /// The content's images, decoded, by the paths the content names them by.
    images: HashMap<String, PremultipliedImage>,
    /// The view's size, once it has one.
    viewport: Option<Viewport>,
    /// Failures of the props and commands, until the app hears of them.
    failures: Vec<String>,
    /// Why the map couldn't be made, once it couldn't.
    refused: Option<String>,
    /// The texture the map draws into, for a host that draws with wgpu.
    texture: Option<MapTexture>,
    epoch: Instant,
}

impl Default for Component {
    fn default() -> Component {
        Component::new()
    }
}

impl Component {
    /// A map of the whole world, made once its view has a size and its
    /// props a style.
    #[must_use]
    pub fn new() -> Component {
        Component::with_inputs(Inputs {
            style: String::new(),
            camera: WORLD,
            content: Content::default(),
            options: Options::default(),
            events: ALL_EVENTS.to_vec(),
        })
    }

    /// A map as `inputs` declares it, made once it has a size.
    #[must_use]
    pub fn with_inputs(inputs: Inputs) -> Component {
        Component {
            inputs,
            engine: None,
            pending: Vec::new(),
            images: HashMap::new(),
            viewport: None,
            failures: Vec::new(),
            refused: None,
            texture: None,
            epoch: Instant::now(),
        }
    }

    /// The map the app declares now, which the next frame takes: a camera
    /// moves the map only when its value changes.
    pub fn set_inputs(&mut self, inputs: Inputs) {
        self.inputs = inputs;
    }

    #[must_use]
    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// Sends the map `command`, which starts at its next frame.
    pub fn send(&mut self, command: Command) {
        let at_ms = self.now_ms();
        match self.engine.as_mut() {
            Some(engine) => engine.command(command, at_ms),
            None => self.pending.push(command),
        }
    }

    /// Gives the map the image the content names `src`, as an encoded file.
    ///
    /// # Errors
    ///
    /// Why the image didn't decode.
    pub fn add_image(&mut self, src: &str, bytes: &[u8]) -> Result<(), String> {
        let image = mln_image::decode_image(bytes).map_err(|e| format!("maplibre: image {src}: {}", e.message))?;
        self.images.insert(src.to_owned(), image);
        if let Some(engine) = self.engine.as_mut() {
            engine.images_changed();
        }
        Ok(())
    }

    /// The map's record: its camera at rest, whether everything in view has
    /// loaded, the last click and context menu, the last failure and the
    /// style's attribution. None before the map is made.
    #[must_use]
    pub fn record(&self) -> Option<Record> {
        self.engine.as_ref().map(Engine::record)
    }

    /// The content's markers, which hold the app's views at places on the
    /// map, as GL JS's `Marker` holds an element.
    #[must_use]
    pub fn markers(&self) -> &[Marker] {
        &self.inputs.content.markers
    }

    /// Where `marker` puts a view laid out at `frame` (x, y, width and height
    /// in points from the map's top-left corner), on the map as its last frame
    /// drew it. A host whose views take a transform applies the homography; one
    /// that only moves views takes its translation, `homography[2]` and
    /// `homography[5]`, where the marker's anchor goes on the view's top-left
    /// corner. None before the map drew a frame, or for a marker that leaves its
    /// view where the layout put it.
    #[must_use]
    pub fn place(&self, marker: &Marker, frame: [f32; 4]) -> Option<Placement> {
        let drawn = self.engine.as_ref()?.drawn_view()?;
        let projection = mln_map::MapProjection::from_view(drawn).ok()?;
        let viewport = self.viewport?;
        let view = View {
            projection: &projection,
            bearing: drawn.camera.bearing,
            pitch: drawn.camera.pitch,
            size: [viewport.size.width as f32, viewport.size.height as f32],
            scale: viewport.scale,
        };
        placement::place(&view, marker, frame)
    }

    /// Where the content's markers at places put their anchors, on the map as
    /// its last frame drew it. A marker without a place stays where the
    /// layout put it, so it has none.
    #[must_use]
    pub fn marker_places(&self) -> Vec<MarkerPlace> {
        self.markers()
            .iter()
            .filter(|marker| marker.lng_lat.is_some())
            .map(|marker| {
                let anchor = marker.anchor.fraction().map(|n| n as f32);
                // A view of no size: the translation is where the anchor goes.
                match self.place(marker, [0.0; 4]) {
                    Some(placement) if !placement.hidden => {
                        let [a, b, x, c, d, y, ..] = placement.homography;
                        MarkerPlace {
                            name: marker.id.clone(),
                            x,
                            y,
                            hidden: false,
                            anchor,
                            rotate: (-b).atan2(a).to_degrees(),
                            squash: a * d - b * c,
                        }
                    }
                    _ => MarkerPlace {
                        name: marker.id.clone(),
                        x: 0.0,
                        y: 0.0,
                        hidden: true,
                        anchor,
                        rotate: 0.0,
                        squash: 1.0,
                    },
                }
            })
            .collect()
    }

    /// Why the map couldn't be made, if it couldn't.
    #[must_use]
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// Lets go of the map and its texture, as when the host's renderer goes
    /// away. The next frame makes them again.
    pub fn release(&mut self) {
        self.engine = None;
        self.texture = None;
    }

    /// Prop `name` changed to `value`, or went with `None`: `style`, a URL or
    /// a document; `camera`, `{"center": [lng, lat], "zoom", "bearing",
    /// "pitch"}`; `content` ([`Content`]) and `options` ([`Options`]), which
    /// may be strings of their JSON; and `events`, the names of the events
    /// to post, all of them without one. A malformed prop fails, which the
    /// next frame posts as an `error`.
    pub fn set_prop(&mut self, name: &str, value: Option<&Json>) {
        let changed = match name {
            "style" => {
                self.inputs.style = match value {
                    Some(Json::String(url)) => url.clone(),
                    Some(document) => document.to_string(),
                    None => String::new(),
                };
                Ok(())
            }
            "camera" => camera(value).map(|camera| self.inputs.camera = camera),
            "content" => Content::parse(&text(value)).map(|content| self.inputs.content = content),
            "options" => Options::parse(&text(value)).map(|options| self.inputs.options = options),
            "events" => events(value).map(|events| self.inputs.events = events),
            _ => Ok(()),
        };
        if let Err(message) = changed {
            self.failures.push(message);
        }
    }

    /// The app called `command(name, args)` on the map's ref, with GL JS's
    /// camera options in `args`.
    pub fn command(&mut self, name: &str, args: &Json) {
        let mut json = match args {
            Json::Object(fields) => Json::Object(fields.clone()),
            Json::Null => Json::Object(serde_json::Map::new()),
            _ => {
                self.failures.push(format!("maplibre: {name}: its options are an object"));
                return;
            }
        };
        json["command"] = Json::from(name);
        match Command::parse(&json.to_string()) {
            Ok(command) => self.send(command),
            Err(message) => self.failures.push(message),
        }
    }

    /// The view's size, once it has one.
    #[must_use]
    pub fn viewport(&self) -> Option<Viewport> {
        self.viewport
    }

    /// The view is `viewport` now, or has no area with `None`.
    pub fn resize(&mut self, viewport: Option<Viewport>) {
        self.viewport = viewport;
        if let (Some(engine), Some(viewport)) = (self.engine.as_mut(), viewport)
            && let Err(message) = engine.resize(viewport)
        {
            self.failures.push(message);
        }
    }

    /// A pointer on the view, which the map follows for clicks and long
    /// presses, and its gestures for moving the map while it is interactive.
    pub fn pointer(&mut self, press: &Press) {
        let at_ms = self.now_ms();
        let Some(engine) = self.engine.as_mut() else { return };
        let Press { id, phase, x, y, button, touch, modifiers } = *press;
        engine.pointer(&Pointer { id, phase, x, y, touch, secondary: button == Button::Secondary, at_ms });
        if !engine.interactive() {
            return;
        }
        let position = ScreenCoordinate::new(x, y);
        let input = match phase {
            Phase::Down => Input::PointerDown {
                id,
                kind: if touch { PointerKind::Touch } else { PointerKind::Mouse },
                button,
                position,
                modifiers,
            },
            Phase::Move => Input::PointerMove { id, position },
            Phase::Up => Input::PointerUp { id, position },
            Phase::Cancel => Input::PointerCancel { id },
        };
        engine.gesture(input, at_ms);
    }

    /// Input other than pointers, such as a wheel or a trackpad's pinch, for
    /// the gestures while the map is interactive.
    pub fn gesture(&mut self, input: Input) {
        let at_ms = self.now_ms();
        if let Some(engine) = self.engine.as_mut()
            && engine.interactive()
        {
            engine.gesture(input, at_ms);
        }
    }

    /// Runs a frame: makes the map once the view has a size and the props a
    /// style, rendering into the target `target` makes for its viewport,
    /// then takes the props and runs the gestures and the map. Returns
    /// whether the map wants another frame, and what to post to the app.
    /// A host runs frames while the map wants them, and again when a prop,
    /// a command, input or the view's size reaches it.
    pub fn frame(
        &mut self,
        target: impl FnOnce(Viewport) -> Result<(RenderTarget, Driver), String>,
    ) -> (bool, Vec<Post>) {
        let now_ms = self.now_ms();
        if self.engine.is_none()
            && self.refused.is_none()
            && !self.inputs.style.is_empty()
            && let Some(viewport) = self.viewport
        {
            let made = target(viewport)
                .and_then(|(target, driver)| Engine::new(viewport, now_ms, self.inputs.clone(), target, driver));
            match made {
                Ok(mut engine) => {
                    for command in self.pending.drain(..) {
                        engine.command(command, now_ms);
                    }
                    self.engine = Some(engine);
                }
                Err(message) => {
                    self.refused = Some(message.clone());
                    self.failures.push(message);
                }
            }
        }
        let mut events = Vec::new();
        let mut again = false;
        if let Some(engine) = self.engine.as_mut() {
            engine.apply_inputs(&self.inputs);
            again = engine.frame(now_ms, &self.images, &mut events);
            if let Some(refusal) = engine.take_refusal() {
                self.failures.push(refusal);
            }
        }
        let mut record = self.engine.as_ref().map(Engine::record).unwrap_or_else(|| Record {
            camera: self.inputs.camera,
            loaded: false,
            click: None,
            contextmenu: None,
            failure: None,
            attribution: String::new(),
        });
        let mut posts = Vec::new();
        let mut post = |event: Event, record: &Record| {
            if self.inputs.events.contains(&event) {
                posts.push(Post { event, name: event.name(), data: serde_json::to_value(record).unwrap_or_default() });
            }
        };
        for event in events {
            post(event, &record);
        }
        for failure in self.failures.drain(..) {
            if let Some(engine) = self.engine.as_mut() {
                engine.report(failure.clone());
            }
            record.failure = Some(failure);
            post(Event::Error, &record);
        }
        (again, posts)
    }

    /// Runs a frame on a host's wgpu `device` for a map of `viewport`: the
    /// map draws into a texture of its own there, which a new frame's
    /// [`TextureFrame::texture`] hands the host to show, and which follows the
    /// viewport's size. The session renders here, on the host's thread, before
    /// the host's own draw goes to `queue`.
    pub fn frame_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, viewport: Viewport) -> TextureFrame {
        let pixels = Size::new(
            ((viewport.size.width as f32 * viewport.scale).round() as u32).max(1),
            ((viewport.size.height as f32 * viewport.scale).round() as u32).max(1),
        );
        self.viewport = Some(viewport);
        let mut shown = None;
        if let (Some(engine), Some(texture)) = (self.engine.as_mut(), self.texture.as_ref())
            && (viewport != engine.viewport() || pixels != texture.pixels())
        {
            let resized = MapTexture::new(device, pixels);
            match engine.set_ring(viewport, resized.ring()) {
                Ok(()) => {
                    shown = Some(resized.texture.clone());
                    self.texture = Some(resized);
                }
                Err(message) => self.failures.push(message),
            }
        }
        let making = self.engine.is_none() && self.refused.is_none() && !self.inputs.style.is_empty();
        let made = making.then(|| MapTexture::new(device, pixels));
        let (again, posts) = self.frame(|viewport| {
            let texture = made.as_ref().ok_or("maplibre: no texture to draw into")?;
            let shared = Device::WebGpu(mln_webgpu::Device::from_wgpu(None, device.clone(), queue.clone(), None));
            Ok((
                RenderTarget::Borrowed { size: viewport.size, ring: texture.ring(), device: shared },
                Driver::CallerThread,
            ))
        });
        if let Some(made) = made
            && self.engine.is_some()
        {
            shown = Some(made.texture.clone());
            self.texture = Some(made);
        }
        TextureFrame { again, posts, texture: shown }
    }

    /// Runs a frame for a map of `viewport` that renders, on a thread of its
    /// own, into a ring of images it shares with the host, of the first of
    /// `kinds` its device makes: IOSurfaces on Apple's platforms, dmabufs on
    /// Linux. A ring of three lets the map render one while the host draws
    /// another and holds a third.
    pub fn frame_shared(&mut self, viewport: Viewport, kinds: &[ExportKind]) -> SharedFrame {
        if self.viewport != Some(viewport) {
            self.resize(Some(viewport));
        }
        let ring = ExportedRing {
            depth: 3,
            kinds: ExportKinds::of(kinds),
            dma_buf_formats: Vec::new(),
            android_hardware_buffer_usage: 0,
            keep_depth: false,
        };
        let (again, posts) = self.frame(|viewport| {
            Ok((RenderTarget::Exported { size: viewport.size, ring, device: None }, Driver::CoreWorker))
        });
        let image = self.engine.as_mut().and_then(Engine::take_frame);
        SharedFrame { again, posts, image }
    }

    fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }
}

/// A target of the `CAMetalLayer` at `layer` for a map of `viewport`, which
/// the map renders into on a thread of its own, on a device that every map
/// of the app shares. Made on the main thread, where wgpu takes the layer.
///
/// # Safety
///
/// `layer` points to a live `CAMetalLayer`, which the surface retains.
///
/// # Errors
///
/// Why the device or the surface couldn't be made.
#[cfg(target_vendor = "apple")]
pub unsafe fn metal_layer_target(
    layer: std::ptr::NonNull<std::ffi::c_void>,
    viewport: Viewport,
) -> Result<(RenderTarget, Driver), String> {
    static DEVICE: std::sync::OnceLock<Result<mln_webgpu::Device, String>> = std::sync::OnceLock::new();
    let device = DEVICE
        .get_or_init(|| mln_webgpu::Device::new(None).map_err(|e| format!("maplibre: {}", e.message)))
        .as_ref()
        .map_err(Clone::clone)?;
    let instance = device.instance().ok_or("maplibre: the device has no instance")?;
    // SAFETY: the caller hands a live `CAMetalLayer`.
    let surface =
        unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer.as_ptr())) }
            .map_err(|e| format!("maplibre: {e}"))?;
    let target = RenderTarget::Surface {
        size: viewport.size,
        surface: mln_runtime::Surface::wgpu(surface),
        device: Some(mln_runtime::Device::WebGpu(device.clone())),
    };
    Ok((target, Driver::CoreWorker))
}

/// A target of the Android `ANativeWindow` at `window` for a map of
/// `viewport`, which the map renders into on a thread of its own and holds
/// a reference to while it does.
///
/// # Safety
///
/// `window` points to a live `ANativeWindow`.
#[cfg(target_os = "android")]
pub unsafe fn android_window_target(
    window: std::ptr::NonNull<std::ffi::c_void>,
    viewport: Viewport,
) -> Result<(RenderTarget, Driver), String> {
    // SAFETY: the caller hands a live window, which the target holds.
    let window = unsafe { android::Window::acquire(window) };
    let target =
        RenderTarget::Surface { size: viewport.size, surface: mln_runtime::Surface::window(window), device: None };
    Ok((target, Driver::CoreWorker))
}

#[cfg(target_os = "android")]
mod android {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use raw_window_handle::{
        AndroidNdkWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
        WindowHandle,
    };

    #[link(name = "android")]
    unsafe extern "C" {
        fn ANativeWindow_acquire(window: *mut c_void);
        fn ANativeWindow_release(window: *mut c_void);
    }

    /// A reference to an `ANativeWindow`, which any thread may use.
    pub(super) struct Window(NonNull<c_void>);

    // SAFETY: an `ANativeWindow` is reference counted and thread safe.
    unsafe impl Send for Window {}
    // SAFETY: as above.
    unsafe impl Sync for Window {}

    impl Window {
        /// # Safety
        ///
        /// `window` points to a live `ANativeWindow`.
        pub(super) unsafe fn acquire(window: NonNull<c_void>) -> Window {
            // SAFETY: the caller hands a live window.
            unsafe { ANativeWindow_acquire(window.as_ptr()) };
            Window(window)
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: the reference `acquire` took.
            unsafe { ANativeWindow_release(self.0.as_ptr()) };
        }
    }

    impl HasWindowHandle for Window {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(self.0));
            // SAFETY: the window lives while this holds it.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    impl HasDisplayHandle for Window {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::android())
        }
    }
}

/// The JSON text of a prop, which may be a document or a string of one.
fn text(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => String::new(),
        Some(Json::String(text)) => text.clone(),
        Some(json) => json.to_string(),
    }
}

/// A `camera` prop: what it leaves out is the whole world's.
fn camera(value: Option<&Json>) -> Result<Camera, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else { return Ok(WORLD) };
    let number = |key: &str, default: f64| match value.get(key) {
        None => Ok(default),
        Some(number) => number.as_f64().ok_or_else(|| format!("maplibre: camera: {key} is a number")),
    };
    let center = match value.get("center") {
        None => WORLD.center,
        Some(center) => serde_json::from_value(center.clone())
            .map_err(|_| "maplibre: camera: center is [longitude, latitude]".to_owned())?,
    };
    Ok(Camera { center, zoom: number("zoom", 0.0)?, bearing: number("bearing", 0.0)?, pitch: number("pitch", 0.0)? })
}

/// An `events` prop: names separated by spaces or commas, or a list of them,
/// and every event without one.
fn events(value: Option<&Json>) -> Result<Vec<Event>, String> {
    match value {
        None | Some(Json::Null) => Ok(ALL_EVENTS.to_vec()),
        Some(Json::String(names)) => Event::parse_list(names),
        Some(Json::Array(names)) => {
            let names: Vec<&str> = names.iter().filter_map(Json::as_str).collect();
            Event::parse_list(&names.join(" "))
        }
        Some(_) => Err("maplibre: events: the events are names".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_declare_the_map() {
        let mut state = Component::new();
        state.set_prop("style", Some(&Json::from("https://tiles.openfreemap.org/styles/liberty")));
        state.set_prop("camera", Some(&serde_json::json!({"center": [8.54, 47.37], "zoom": 11})));
        state.set_prop("events", Some(&serde_json::json!(["load", "click"])));
        state.set_prop("options", Some(&serde_json::json!({"maxZoom": 16})));
        assert!(state.failures.is_empty(), "{:?}", state.failures);
        assert_eq!(state.inputs.style, "https://tiles.openfreemap.org/styles/liberty");
        assert_eq!(state.inputs.camera, Camera { center: [8.54, 47.37], zoom: 11.0, bearing: 0.0, pitch: 0.0 });
        assert_eq!(state.inputs.events, vec![Event::Load, Event::Click]);
        assert_eq!(state.inputs.options.max_zoom, Some(16.0));
        state.set_prop("events", None);
        assert_eq!(state.inputs.events, ALL_EVENTS.to_vec());
    }

    #[test]
    fn markers_at_places_are_hidden_until_the_map_draws() {
        let mut state = Component::new();
        let markers =
            serde_json::json!({"markers": {"station": {"lngLat": [8.54, 47.38], "anchor": "bottom"}, "compass": {}}});
        state.set_prop("content", Some(&markers));
        let places = state.marker_places();
        assert_eq!(places.len(), 1, "a marker without a place stays where the layout put it");
        assert_eq!(places[0].name, "station");
        assert!(places[0].hidden);
        assert_eq!(places[0].anchor, [0.5, 1.0]);
    }

    #[cfg(all(feature = "metal", target_vendor = "apple"))]
    #[test]
    fn a_map_renders_into_an_iosurface_it_shares() {
        use mln_runtime::{ExportedImage, FrameTextureKind};
        let mut state = Component::new();
        let style = r##"{"version": 8, "sources": {}, "layers": [{"id": "bg", "type": "background", "paint": {"background-color": "#ff0000"}}]}"##;
        state.set_prop("style", Some(&Json::from(style)));
        let viewport = Viewport { size: Size::new(64, 48), scale: 2.0 };
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        let image = loop {
            let frame = state.frame_shared(viewport, &[ExportKind::IoSurface]);
            assert!(state.refused().is_none(), "{:?}", state.refused());
            if let Some(image) = frame.image {
                break image;
            }
            assert!(Instant::now() < deadline, "no frame came");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let texture = image.texture().expect("a shared frame has its image");
        assert_eq!(texture.size, Size::new(128, 96));
        assert!(matches!(texture.kind, FrameTextureKind::Exported(ExportedImage::IoSurface { .. })));
    }

    #[test]
    fn a_malformed_prop_is_a_failure_the_app_hears_of() {
        let mut state = Component::new();
        state.set_prop("camera", Some(&serde_json::json!({"zoom": "near"})));
        state.set_prop("events", Some(&Json::from("load nothing")));
        let (again, posts) = state.frame(|_| Err("no target".to_owned()));
        assert!(!again);
        let failures: Vec<_> = posts.iter().map(|post| (post.name, post.data["failure"].as_str())).collect();
        assert_eq!(
            failures,
            [
                ("error", Some("maplibre: camera: zoom is a number")),
                (
                    "error",
                    Some(
                        "maplibre: events: nothing is no event; the map posts load, moveend, click, contextmenu and error"
                    )
                )
            ]
        );
    }

    #[test]
    fn commands_wait_for_the_map() {
        let mut state = Component::new();
        state.command("flyTo", &serde_json::json!({"center": [8.54, 47.37], "zoom": 14}));
        state.command("jumpTo", &Json::from(3));
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.failures, ["maplibre: jumpTo: its options are an object"]);
    }
}
