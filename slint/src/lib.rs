//! MapLibre maps in Slint apps, drawn by mln-rs on Slint's wgpu renderer
//! through maplibre-rust-ui's engine.
//!
//! An app imports `MapLibreMap` and the `MapLibre` global from
//! `@maplibre/maplibre.slint`, places the map in its UI and exports the
//! global. Its build script gives the Slint compiler the library path this
//! crate's build hands it as `DEP_MAPLIBRE_SLINT_UI`. In Rust the app makes a
//! [`SlintMap`] with the map it declares ([`model::Inputs`]) and connects it:
//!
//! ```ignore
//! slint::BackendSelector::new().require_wgpu_30(maplibre_slint::wgpu_configuration()).select()?;
//! let app = App::new()?;
//! let map = std::rc::Rc::new(std::cell::RefCell::new(maplibre_slint::SlintMap::new(inputs)));
//! maplibre_slint::connect!(app, map)?;
//! app.run()?;
//! ```
//!
//! The map renders before each of the window's frames, on Slint's device,
//! into a texture of its own that Slint then draws, and asks for frames while
//! it loads, animates or follows a gesture. Its animations run on a clock that
//! starts with the map.

use std::collections::HashMap;
use std::time::Instant;

use maplibre_rust_ui::gestures::{Button, Input, Modifiers, PointerKind, WheelDelta};
pub use maplibre_rust_ui::model;
use maplibre_rust_ui::model::{Command, Event, Inputs, Record};
use maplibre_rust_ui::runtime::{Device, Driver, RenderTarget};
use maplibre_rust_ui::texture::MapTexture;
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::image::PremultipliedImage;
use maplibre_rust_ui::util::size::Size;
use maplibre_rust_ui::{Engine, Phase, Pointer, Viewport};
use slint::wgpu_30::wgpu;

/// The wgpu configuration for the app to select, so that Slint's device runs
/// mln-rs's shaders: WebGPU's default limits, where Slint asks for WebGL 2's.
#[must_use]
pub fn wgpu_configuration() -> slint::wgpu_30::WGPUConfiguration {
    let mut settings = slint::wgpu_30::WGPUSettings::default();
    settings.device_required_limits = wgpu::Limits::default();
    slint::wgpu_30::WGPUConfiguration::Automatic(settings)
}

/// A map in a Slint window, and what the app knows of it.
pub struct SlintMap {
    inputs: Inputs,
    engine: Option<Engine>,
    texture: Option<MapTexture>,
    /// Commands sent before the map was made.
    pending: Vec<Command>,
    /// The content's images, decoded, by the paths the content names them by.
    images: HashMap<String, PremultipliedImage>,
    events: Vec<Event>,
    record: Option<Record>,
    error: Option<String>,
    epoch: Instant,
}

/// What a frame did: the image to show, when the map has a new texture, and
/// whether the map wants another frame.
pub struct Rendered {
    pub image: Option<slint::Image>,
    pub again: bool,
}

impl SlintMap {
    /// A map as `inputs` declares it, made at the window's first frame.
    #[must_use]
    pub fn new(inputs: Inputs) -> SlintMap {
        SlintMap {
            inputs,
            engine: None,
            texture: None,
            pending: Vec::new(),
            images: HashMap::new(),
            events: Vec::new(),
            record: None,
            error: None,
            epoch: Instant::now(),
        }
    }

    /// The map the app declares now. The map takes what changed at its next
    /// frame: a camera moves only when its value changes.
    pub fn set_inputs(&mut self, inputs: Inputs) {
        self.inputs = inputs;
    }

    #[must_use]
    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// Sends the map `command`, which starts at its next frame.
    pub fn command(&mut self, command: Command) {
        match self.engine.as_mut() {
            Some(engine) => engine.command(command, self.epoch.elapsed().as_secs_f64() * 1000.0),
            None => self.pending.push(command),
        }
    }

    /// Gives the map the image the content names `src`, as an encoded file.
    ///
    /// # Errors
    ///
    /// Why the image didn't decode.
    pub fn add_image(&mut self, src: &str, bytes: &[u8]) -> Result<(), String> {
        let image = maplibre_rust_ui::image::decode_image(bytes)
            .map_err(|e| format!("maplibre: image {src}: {}", e.message))?;
        self.images.insert(src.to_owned(), image);
        if let Some(engine) = self.engine.as_mut() {
            engine.images_changed();
        }
        Ok(())
    }

    /// A pointer, as `MapLibre.pointer` reports it: its kind (0 down, 1 up,
    /// 2 move, 3 cancel), its button (0 the primary, 1 the middle, 2 the
    /// secondary), its finger (0 for a mouse), where it is in logical pixels,
    /// and the shift, control, alt and meta keys.
    #[allow(clippy::too_many_arguments)]
    pub fn pointer(&mut self, kind: i32, button: i32, finger: i32, x: f32, y: f32, keys: [bool; 4]) {
        let at_ms = self.now_ms();
        let Some(engine) = self.engine.as_mut() else { return };
        let phase = match kind {
            0 => Phase::Down,
            1 => Phase::Up,
            2 => Phase::Move,
            _ => Phase::Cancel,
        };
        let id = u32::try_from(finger).unwrap_or(0);
        let (x, y) = (f64::from(x), f64::from(y));
        let touch = finger != 0;
        engine.pointer(&Pointer { id, phase, x, y, touch, secondary: button == 2, at_ms });
        if !engine.interactive() {
            return;
        }
        let position = ScreenCoordinate::new(x, y);
        let input = match phase {
            Phase::Down => Input::PointerDown {
                id,
                kind: if touch { PointerKind::Touch } else { PointerKind::Mouse },
                button: match button {
                    2 => Button::Secondary,
                    1 => Button::Middle,
                    _ => Button::Primary,
                },
                position,
                modifiers: modifiers(keys),
            },
            Phase::Move => Input::PointerMove { id, position },
            Phase::Up => Input::PointerUp { id, position },
            Phase::Cancel => Input::PointerCancel { id },
        };
        engine.gesture(input, at_ms);
    }

    /// A wheel or trackpad scroll, as `MapLibre.scroll` reports it, by
    /// Slint's sign: up and left are positive.
    pub fn scroll(&mut self, dx: f32, dy: f32, x: f32, y: f32, keys: [bool; 4]) {
        let at_ms = self.now_ms();
        let Some(engine) = self.engine.as_mut() else { return };
        let _ = dx;
        if engine.interactive() {
            let input = Input::Wheel {
                position: ScreenCoordinate::new(f64::from(x), f64::from(y)),
                // The web's sign, which the gestures take: down is positive.
                delta: WheelDelta::Pixels(-f64::from(dy)),
                modifiers: modifiers(keys),
            };
            engine.gesture(input, at_ms);
        }
    }

    /// The events since the last call, of those the app listens for.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The map's record: its camera at rest, whether everything in view has
    /// loaded, the last click and context menu, the last failure and the
    /// style's attribution.
    #[must_use]
    pub fn record(&self) -> Option<&Record> {
        self.record.as_ref()
    }

    /// Why the map couldn't be made or draw, if it couldn't.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Renders a frame for a map of `size` logical pixels at `scale`, on
    /// Slint's `device`, before Slint renders the window.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: (f32, f32), scale: f32) -> Rendered {
        let now_ms = self.now_ms();
        let logical = Size::new((size.0.round() as u32).max(1), (size.1.round() as u32).max(1));
        let viewport = Viewport { size: logical, scale };
        let pixels = Size::new(((size.0 * scale).round() as u32).max(1), ((size.1 * scale).round() as u32).max(1));
        let mut image = None;
        if self.engine.is_none() {
            let texture = MapTexture::new(device, pixels);
            let shared =
                Device::WebGpu(maplibre_rust_ui::webgpu::Device::from_wgpu(None, device.clone(), queue.clone(), None));
            let target = RenderTarget::Borrowed { size: logical, ring: texture.ring(), device: shared };
            match Engine::new(viewport, now_ms, self.inputs.clone(), target, Driver::CallerThread) {
                Ok(mut engine) => {
                    for command in self.pending.drain(..) {
                        engine.command(command, now_ms);
                    }
                    image = slint::Image::try_from(texture.texture.clone()).ok();
                    self.engine = Some(engine);
                    self.texture = Some(texture);
                }
                Err(message) => {
                    self.error = Some(message);
                    return Rendered { image: None, again: false };
                }
            }
        }
        let (Some(engine), Some(texture)) = (self.engine.as_mut(), self.texture.as_mut()) else {
            return Rendered { image: None, again: false };
        };
        if viewport != engine.viewport() || pixels != texture.pixels() {
            let resized = MapTexture::new(device, pixels);
            match engine.set_ring(viewport, resized.ring()) {
                Ok(()) => {
                    image = slint::Image::try_from(resized.texture.clone()).ok();
                    *texture = resized;
                }
                Err(message) => self.error = Some(message),
            }
        }
        engine.apply_inputs(&self.inputs);
        let mut events = Vec::new();
        let again = engine.frame(now_ms, &self.images, &mut events);
        self.events.extend(events.into_iter().filter(|event| self.inputs.events.contains(event)));
        self.record = Some(engine.record());
        if let Some(refusal) = engine.take_refusal() {
            self.error = Some(refusal);
        }
        Rendered { image, again }
    }

    /// Lets go of the map, as when the window's renderer goes away.
    pub fn release(&mut self) {
        self.engine = None;
        self.texture = None;
    }

    fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }
}

fn modifiers([shift, control, alt, meta]: [bool; 4]) -> Modifiers {
    Modifiers { shift, ctrl: control, alt, meta }
}

/// Connects a [`SlintMap`] in an `Rc<RefCell<…>>` to the app's component,
/// whose `MapLibre` global (exported from `@maplibre/maplibre.slint`) must
/// be in scope: the map renders before each of the window's frames and shows
/// them through `MapLibre.frame`, and takes the input `MapLibreMap` reports.
/// Evaluates to the result of setting the window's rendering notifier.
#[macro_export]
macro_rules! connect {
    ($app:expr, $map:expr) => {{
        use ::slint::ComponentHandle as _;
        let app = &$app;
        let map: ::std::rc::Rc<::std::cell::RefCell<$crate::SlintMap>> = ::std::rc::Rc::clone(&$map);
        let global = app.global::<MapLibre>();
        {
            let (map, weak) = (map.clone(), app.as_weak());
            global.on_pointer(move |kind, button, finger, x, y, shift, control, alt, meta| {
                map.borrow_mut().pointer(kind, button, finger, x, y, [shift, control, alt, meta]);
                if let Some(app) = weak.upgrade() {
                    app.window().request_redraw();
                }
            });
        }
        {
            let (map, weak) = (map.clone(), app.as_weak());
            global.on_scroll(move |dx, dy, x, y, shift, control, alt, meta| {
                map.borrow_mut().scroll(dx, dy, x, y, [shift, control, alt, meta]);
                if let Some(app) = weak.upgrade() {
                    app.window().request_redraw();
                }
            });
        }
        let weak = app.as_weak();
        // A redraw asked for while the window renders is lost, so the map
        // asks for its next frame once the window has rendered.
        let again = ::std::cell::Cell::new(false);
        app.window().set_rendering_notifier(move |state, graphics| {
            let Some(app) = weak.upgrade() else { return };
            match (state, graphics) {
                (::slint::RenderingState::BeforeRendering, ::slint::GraphicsAPI::WGPU30 { device, queue, .. }) => {
                    let global = app.global::<MapLibre>();
                    let size = (global.get_width(), global.get_height());
                    let scale = app.window().scale_factor();
                    let rendered = map.borrow_mut().render(device, queue, size, scale);
                    if let Some(image) = rendered.image {
                        global.set_frame(image);
                    }
                    again.set(rendered.again);
                }
                (::slint::RenderingState::AfterRendering, _) => {
                    if again.take() {
                        app.window().request_redraw();
                    }
                }
                (::slint::RenderingState::RenderingTeardown, _) => map.borrow_mut().release(),
                _ => {}
            }
        })
    }};
}
