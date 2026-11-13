//! MapLibre maps in [GPUI](https://www.gpui.rs) apps, drawn by mln-rs through
//! maplibre-rust-ui's engine into IOSurfaces that GPUI draws as they are.
//!
//! A [`GpuiMap`] is an entity whose view is the map, with the map the app
//! declares ([`model::Inputs`]). Its view fills the element it is in:
//!
//! ```ignore
//! let map = cx.new(|_| GpuiMap::new(inputs));
//! cx.subscribe(&map, |app, map, event: &Event, cx| {
//!     // a click's features are in map.read(cx).record()
//! })
//! .detach();
//! div().size_full().child(map.clone())
//! ```
//!
//! The map renders on a thread of its own into a ring of IOSurfaces, and the
//! view hands GPUI the newest as a `CVPixelBuffer`, which GPUI's Metal
//! renderer samples where the view is: nothing is copied. A frame goes back to
//! the map once GPUI let go of it. The view takes the mouse with its modifier
//! keys, a trackpad's scroll and pinch, and asks for frames while the map
//! loads, animates or follows a gesture.

use std::collections::VecDeque;
use std::ptr::NonNull;

use gpui::{
    App, Bounds, Context, DispatchPhase, EventEmitter, Hitbox, HitboxBehavior, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PinchEvent, Pixels, Render, ScrollDelta, ScrollEvent,
    Styled, TouchPhase, Window, canvas, div, px,
};
use maplibre_rust_ui::component::Component;
pub use maplibre_rust_ui::component::{MarkerPlace, Press};
pub use maplibre_rust_ui::gestures::{Button, Modifiers};
use maplibre_rust_ui::gestures::{Input, PinchPhase, WheelDelta};
pub use maplibre_rust_ui::model;
use maplibre_rust_ui::model::{Command, Event, Inputs, Record};
use maplibre_rust_ui::runtime::{AcquiredFrame, ExportKind, ExportedImage, FrameTextureKind};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::size::Size;
pub use maplibre_rust_ui::{Phase, Viewport};
use objc2_core_foundation::CFRetained;
use objc2_core_video::{CVPixelBuffer, CVPixelBufferCreateWithIOSurface, kCVReturnSuccess};
use objc2_io_surface::IOSurfaceRef;

/// A map in a GPUI app, and what the app knows of it.
pub struct GpuiMap {
    component: Component,
    record: Option<Record>,
    places: Vec<MarkerPlace>,
    /// The frame GPUI draws, and frames it drew before, which it may still be
    /// reading, with the paints since.
    shown: Option<Shown>,
    held: VecDeque<(Shown, u32)>,
    /// The mouse button held down on the map, which it follows until it lifts.
    pressed: Option<MouseButton>,
    pinching: bool,
}

impl EventEmitter<Event> for GpuiMap {}

/// A frame of the map: its IOSurface as a buffer GPUI draws, and the frame,
/// which goes back to the map as it drops, after the buffer.
struct Shown {
    buffer: CFRetained<CVPixelBuffer>,
    _frame: AcquiredFrame,
}

impl Shown {
    fn of(frame: AcquiredFrame) -> Option<Shown> {
        let texture = frame.texture()?;
        let FrameTextureKind::Exported(ExportedImage::IoSurface { surface }) = &texture.kind else { return None };
        let surface = NonNull::new(*surface as usize as *mut IOSurfaceRef)?;
        let mut buffer = std::ptr::null_mut();
        // SAFETY: the frame keeps its IOSurface while it lives, and the buffer
        // retains it too.
        let status =
            unsafe { CVPixelBufferCreateWithIOSurface(None, surface.as_ref(), None, NonNull::from(&mut buffer)) };
        if status != kCVReturnSuccess {
            return None;
        }
        // SAFETY: a successful create hands over the buffer it made.
        let buffer = unsafe { CFRetained::from_raw(NonNull::new(buffer)?) };
        Some(Shown { buffer, _frame: frame })
    }

    /// Whether GPUI let go of the buffer: its scene and its textures of it
    /// hold it until the GPU read it. A frame two paints back, which GPUI's
    /// renderer has long drawn, goes back regardless.
    fn released(&self, paints: u32) -> bool {
        (paints >= 1 && self.buffer.retain_count() == 1) || paints >= 3
    }
}

impl GpuiMap {
    /// A map as `inputs` declares it, made at its view's first paint.
    #[must_use]
    pub fn new(inputs: Inputs) -> GpuiMap {
        GpuiMap {
            component: Component::with_inputs(inputs),
            record: None,
            places: Vec::new(),
            shown: None,
            held: VecDeque::new(),
            pressed: None,
            pinching: false,
        }
    }

    /// The map the app declares now. The map takes what changed at its next
    /// frame: a camera moves only when its value changes.
    pub fn set_inputs(&mut self, inputs: Inputs, cx: &mut Context<Self>) {
        self.component.set_inputs(inputs);
        cx.notify();
    }

    #[must_use]
    pub fn inputs(&self) -> &Inputs {
        self.component.inputs()
    }

    /// Sends the map `command`, which starts at its next frame.
    pub fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        self.component.send(command);
        cx.notify();
    }

    /// Gives the map the image the content names `src`, as an encoded file.
    ///
    /// # Errors
    ///
    /// Why the image didn't decode.
    pub fn add_image(&mut self, src: &str, bytes: &[u8], cx: &mut Context<Self>) -> Result<(), String> {
        cx.notify();
        self.component.add_image(src, bytes)
    }

    /// The map's record: its camera at rest, whether everything in view has
    /// loaded, the last click and context menu, the last failure and the
    /// style's attribution.
    #[must_use]
    pub fn record(&self) -> Option<&Record> {
        self.record.as_ref()
    }

    /// Where the content's markers at places go, on the map as it last drew,
    /// for [`at_marker`]. The map notifies its observers when one moved.
    #[must_use]
    pub fn marker_places(&self) -> &[MarkerPlace] {
        &self.places
    }

    /// The map's size in points and its pixels a point, once its view painted.
    #[must_use]
    pub fn viewport(&self) -> Option<Viewport> {
        self.component.viewport()
    }

    /// Why the map couldn't be made, if it couldn't.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.component.refused()
    }

    /// A pointer on the map, at points from its top-left corner, as the
    /// mouse's are: for input of the app's own.
    pub fn pointer(&mut self, press: &Press, cx: &mut Context<Self>) {
        self.component.pointer(press);
        cx.notify();
    }

    /// Runs the map's frame for a view at `bounds`, and paints its newest
    /// image there.
    fn paint(&mut self, bounds: Bounds<Pixels>, hitbox: &Hitbox, window: &mut Window, cx: &mut Context<Self>) {
        let size = |pixels: Pixels| (f32::from(pixels).round() as u32).max(1);
        let viewport = Viewport {
            size: Size::new(size(bounds.size.width), size(bounds.size.height)),
            scale: window.scale_factor(),
        };
        let frame = self.component.frame_shared(viewport, &[ExportKind::IoSurface]);
        self.record = self.component.record();
        for post in &frame.posts {
            cx.emit(post.event);
        }
        let places = self.component.marker_places();
        if places != self.places {
            self.places = places;
            cx.notify();
        }
        for (_, paints) in &mut self.held {
            *paints += 1;
        }
        self.held.retain(|(shown, paints)| !shown.released(*paints));
        if let Some(shown) = frame.image.and_then(Shown::of)
            && let Some(older) = self.shown.replace(shown)
        {
            self.held.push_back((older, 0));
        }
        if let Some(shown) = &self.shown {
            window.paint_surface(bounds, shown.buffer.clone());
        }
        if frame.again {
            window.request_animation_frame();
        }
        listen(cx.entity(), hitbox.clone(), window);
    }

    fn press(&mut self, phase: Phase, at: ScreenCoordinate, button: MouseButton, keys: gpui::Modifiers) {
        let button = match button {
            MouseButton::Left => Button::Primary,
            MouseButton::Right => Button::Secondary,
            MouseButton::Middle => Button::Middle,
            MouseButton::Navigate(_) => return,
        };
        let press = Press { id: 0, phase, x: at.x, y: at.y, button, touch: false, modifiers: modifiers(keys) };
        self.component.pointer(&press);
    }
}

impl Render for GpuiMap {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let map = cx.entity();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, cx| map.update(cx, |map, cx| map.paint(bounds, &hitbox, window, cx)),
        )
        .size_full()
    }
}

/// Hands the map the mouse over `hitbox`, the buttons it holds wherever they
/// go, and the trackpad's scroll and pinch.
fn listen(map: gpui::Entity<GpuiMap>, hitbox: Hitbox, window: &mut Window) {
    let origin = hitbox.bounds.origin;
    let at = move |position: gpui::Point<Pixels>| {
        let at = position - origin;
        ScreenCoordinate::new(f64::from(f32::from(at.x)), f64::from(f32::from(at.y)))
    };
    window.on_mouse_event({
        let (map, hitbox) = (map.clone(), hitbox.clone());
        move |event: &MouseDownEvent, phase, window, cx: &mut App| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            map.update(cx, |map, cx| {
                if map.pressed.is_none() {
                    map.pressed = Some(event.button);
                    map.press(Phase::Down, at(event.position), event.button, event.modifiers);
                    cx.notify();
                }
            });
        }
    });
    window.on_mouse_event({
        let map = map.clone();
        move |event: &MouseMoveEvent, phase, _, cx: &mut App| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            map.update(cx, |map, cx| {
                if let Some(button) = map.pressed {
                    map.press(Phase::Move, at(event.position), button, event.modifiers);
                    cx.notify();
                }
            });
        }
    });
    window.on_mouse_event({
        let map = map.clone();
        move |event: &MouseUpEvent, phase, _, cx: &mut App| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            map.update(cx, |map, cx| {
                if map.pressed == Some(event.button) {
                    map.pressed = None;
                    map.press(Phase::Up, at(event.position), event.button, event.modifiers);
                    cx.notify();
                }
            });
        }
    });
    window.on_mouse_event({
        let (map, hitbox) = (map.clone(), hitbox.clone());
        move |event: &ScrollEvent, phase, window, cx: &mut App| {
            if phase != DispatchPhase::Bubble || !hitbox.should_handle_scroll(window) {
                return;
            }
            // The web's sign, which the map's gestures take: down is positive.
            let delta = match event.delta {
                ScrollDelta::Pixels(delta) => WheelDelta::Pixels(-f64::from(f32::from(delta.y))),
                ScrollDelta::Lines(delta) => WheelDelta::Notches(-f64::from(delta.y)),
            };
            let input = Input::Wheel { position: at(event.position), delta, modifiers: modifiers(event.modifiers) };
            map.update(cx, |map, cx| {
                map.component.gesture(input);
                cx.notify();
            });
        }
    });
    window.on_mouse_event(move |event: &PinchEvent, phase, window, cx: &mut App| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        map.update(cx, |map, cx| {
            let phase = match event.phase {
                TouchPhase::Started if hitbox.is_hovered(window) => PinchPhase::Begin,
                TouchPhase::Moved if map.pinching => PinchPhase::Update,
                TouchPhase::Ended if map.pinching => PinchPhase::End,
                TouchPhase::Cancelled if map.pinching => PinchPhase::Cancel,
                _ => return,
            };
            map.pinching = matches!(phase, PinchPhase::Begin | PinchPhase::Update);
            let scale = 1.0 + f64::from(event.delta);
            map.component.gesture(Input::Pinch { center: at(event.position), scale, rotation: 0.0, phase });
            cx.notify();
        });
    });
}

fn modifiers(keys: gpui::Modifiers) -> Modifiers {
    Modifiers { shift: keys.shift, ctrl: keys.control, alt: keys.alt, meta: keys.platform }
}

/// Holds `child` at a marker's place, in an element the map fills, upright:
/// its anchor on the point, and hidden while the place is. GPUI turns no
/// elements, so a marker that turns with the map stays upright here.
pub fn at_marker(place: &MarkerPlace, child: impl IntoElement) -> impl IntoElement {
    // A box the child sits in by its anchor, larger than any marker, so that
    // flex alignment puts the anchor on the point whatever the child's size.
    const REACH: f32 = 2000.0;
    let [ax, ay] = place.anchor;
    let placed =
        div().absolute().left(px(place.x - ax * REACH)).top(px(place.y - ay * REACH)).w(px(REACH)).h(px(REACH)).flex();
    let placed = match ax {
        x if x <= 0.0 => placed.justify_start(),
        x if x >= 1.0 => placed.justify_end(),
        _ => placed.justify_center(),
    };
    let placed = match ay {
        y if y <= 0.0 => placed.items_start(),
        y if y >= 1.0 => placed.items_end(),
        _ => placed.items_center(),
    };
    placed.opacity(if place.hidden { 0.0 } else { 1.0 }).child(child)
}
