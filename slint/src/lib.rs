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

pub use maplibre_rust_ui::component::MarkerPlace;
use maplibre_rust_ui::component::{Component, Press};
use maplibre_rust_ui::gestures::{Button, Input, Modifiers, WheelDelta};
pub use maplibre_rust_ui::model;
use maplibre_rust_ui::model::{Command, Event, Inputs, Record};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::size::Size;
use maplibre_rust_ui::{Phase, Viewport};
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
    component: Component,
    events: Vec<Event>,
    record: Option<Record>,
    places: Vec<MarkerPlace>,
}

/// What a frame did: the image to show, when the map has a new texture, and
/// whether the map wants another frame.
pub struct Rendered {
    pub image: Option<slint::Image>,
    pub again: bool,
    /// The places of the content's markers, when one moved.
    pub places: Option<Vec<MarkerPlace>>,
}

impl SlintMap {
    /// A map as `inputs` declares it, made at the window's first frame.
    #[must_use]
    pub fn new(inputs: Inputs) -> SlintMap {
        SlintMap { component: Component::with_inputs(inputs), events: Vec::new(), record: None, places: Vec::new() }
    }

    /// The map the app declares now. The map takes what changed at its next
    /// frame: a camera moves only when its value changes.
    pub fn set_inputs(&mut self, inputs: Inputs) {
        self.component.set_inputs(inputs);
    }

    #[must_use]
    pub fn inputs(&self) -> &Inputs {
        self.component.inputs()
    }

    /// Sends the map `command`, which starts at its next frame.
    pub fn command(&mut self, command: Command) {
        self.component.send(command);
    }

    /// Gives the map the image the content names `src`, as an encoded file.
    ///
    /// # Errors
    ///
    /// Why the image didn't decode.
    pub fn add_image(&mut self, src: &str, bytes: &[u8]) -> Result<(), String> {
        self.component.add_image(src, bytes)
    }

    /// A pointer, as `MapLibre.pointer` reports it: its kind (0 down, 1 up,
    /// 2 move, 3 cancel), its button (0 the primary, 1 the middle, 2 the
    /// secondary), its finger (0 for a mouse), where it is in logical pixels,
    /// and the shift, control, alt and meta keys.
    #[allow(clippy::too_many_arguments)]
    pub fn pointer(&mut self, kind: i32, button: i32, finger: i32, x: f32, y: f32, keys: [bool; 4]) {
        let phase = match kind {
            0 => Phase::Down,
            1 => Phase::Up,
            2 => Phase::Move,
            _ => Phase::Cancel,
        };
        let button = match button {
            2 => Button::Secondary,
            1 => Button::Middle,
            _ => Button::Primary,
        };
        self.component.pointer(&Press {
            id: u32::try_from(finger).unwrap_or(0),
            phase,
            x: f64::from(x),
            y: f64::from(y),
            button,
            touch: finger != 0,
            modifiers: modifiers(keys),
        });
    }

    /// A wheel or trackpad scroll, as `MapLibre.scroll` reports it, by
    /// Slint's sign: up and left are positive.
    pub fn scroll(&mut self, dx: f32, dy: f32, x: f32, y: f32, keys: [bool; 4]) {
        let _ = dx;
        self.component.gesture(Input::Wheel {
            position: ScreenCoordinate::new(f64::from(x), f64::from(y)),
            // The web's sign, which the gestures take: down is positive.
            delta: WheelDelta::Pixels(-f64::from(dy)),
            modifiers: modifiers(keys),
        });
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

    /// Why the map couldn't be made, if it couldn't.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.component.refused()
    }

    /// Renders a frame for a map of `size` logical pixels at `scale`, on
    /// Slint's `device`, before Slint renders the window.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: (f32, f32), scale: f32) -> Rendered {
        let logical = Size::new((size.0.round() as u32).max(1), (size.1.round() as u32).max(1));
        let frame = self.component.frame_texture(device, queue, Viewport { size: logical, scale });
        self.events.extend(frame.posts.iter().map(|post| post.event));
        self.record = self.component.record();
        let places = self.component.marker_places();
        let places = (places != self.places).then(|| {
            self.places.clone_from(&places);
            places
        });
        Rendered {
            image: frame.texture.and_then(|texture| slint::Image::try_from(texture).ok()),
            again: frame.again,
            places,
        }
    }

    /// Lets go of the map, as when the window's renderer goes away.
    pub fn release(&mut self) {
        self.component.release();
    }
}

fn modifiers([shift, control, alt, meta]: [bool; 4]) -> Modifiers {
    Modifiers { shift, ctrl: control, alt, meta }
}

/// Connects a [`SlintMap`] in an `Rc<RefCell<…>>` to the app's component,
/// whose `MapLibre` global and `MapLibreMarkerPlace` struct (exported from
/// `@maplibre/maplibre.slint`) must be in scope: the map renders before each
/// of the window's frames and shows them through `MapLibre.frame`, sets
/// `MapLibre.places` when a marker moved, and takes the input `MapLibreMap`
/// reports.
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
                    if let Some(places) = rendered.places {
                        let places: ::std::vec::Vec<MapLibreMarkerPlace> = places
                            .into_iter()
                            .map(|place| MapLibreMarkerPlace {
                                name: place.name.into(),
                                x: place.x,
                                y: place.y,
                                hidden: place.hidden,
                                anchor_x: place.anchor[0],
                                anchor_y: place.anchor[1],
                                rotation: place.rotate,
                            })
                            .collect();
                        global.set_places(::slint::ModelRc::new(::slint::VecModel::from(places)));
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
