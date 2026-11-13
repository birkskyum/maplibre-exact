//! MapLibre maps in [egui](https://www.egui.rs) apps, drawn by mln-rs on
//! egui's wgpu renderer through maplibre-rust-ui's engine.
//!
//! An app keeps an [`EguiMap`] with the map it declares ([`model::Inputs`])
//! and shows it in a frame's UI with the renderer's state, which eframe hands
//! out as `frame.wgpu_render_state()`:
//!
//! ```ignore
//! let render_state = frame.wgpu_render_state().expect("eframe renders with wgpu");
//! egui::CentralPanel::default().show(ctx, |ui| {
//!     self.map.show(ui, render_state);
//! });
//! ```
//!
//! The map draws into a texture of its own on egui's device, before egui
//! draws the frame, and asks for frames while it loads, animates or follows a
//! gesture. It takes the mouse, a trackpad's scroll, pinch and rotation, and
//! touches over its area.

use std::collections::HashMap;

use egui_wgpu::RenderState;
use maplibre_rust_ui::component::{Component, Press};
use maplibre_rust_ui::gestures::{Button, Input, Modifiers, PinchPhase, WheelDelta};
pub use maplibre_rust_ui::model;
use maplibre_rust_ui::model::{Command, Event, Inputs, Record};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::size::Size;
use maplibre_rust_ui::{Phase, Viewport, wgpu};

/// A map in an egui UI, and what the app knows of it.
pub struct EguiMap {
    component: Component,
    /// The map's texture, once egui's renderer has it.
    texture: Option<egui::TextureId>,
    events: Vec<Event>,
    record: Option<Record>,
    /// The mouse buttons pressed on the map, which it follows until they lift.
    buttons: Vec<egui::PointerButton>,
    /// The touches on the map, by egui's ids, with the map's.
    touches: HashMap<u64, u32>,
    next_touch: u32,
    /// Whether a trackpad pinch or rotation goes on.
    pinching: bool,
    /// Where the map was shown last.
    rect: Option<egui::Rect>,
}

impl EguiMap {
    /// A map as `inputs` declares it, made at its first frame.
    #[must_use]
    pub fn new(inputs: Inputs) -> EguiMap {
        EguiMap {
            component: Component::with_inputs(inputs),
            texture: None,
            events: Vec::new(),
            record: None,
            buttons: Vec::new(),
            touches: HashMap::new(),
            next_touch: 1,
            pinching: false,
            rect: None,
        }
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

    /// Where the map was shown last, in the UI's points.
    #[must_use]
    pub fn rect(&self) -> Option<egui::Rect> {
        self.rect
    }

    /// Why the map couldn't be made, if it couldn't.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.component.refused()
    }

    /// Shows the map in the rest of `ui`, takes its input, and draws its frame
    /// on the device of `render_state`.
    pub fn show(&mut self, ui: &mut egui::Ui, render_state: &RenderState) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.rect = Some(rect);
        self.input(ui, rect);
        let viewport = Viewport {
            size: Size::new(rect.width().round().max(1.0) as u32, rect.height().round().max(1.0) as u32),
            scale: ui.ctx().pixels_per_point(),
        };
        let frame = self.component.frame_texture(&render_state.device, &render_state.queue, viewport);
        self.events.extend(frame.posts.iter().map(|post| post.event));
        self.record = self.component.record();
        if let Some(texture) = frame.texture {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut renderer = render_state.renderer.write();
            match self.texture {
                Some(id) => {
                    renderer.update_egui_texture_from_wgpu_texture(
                        &render_state.device,
                        &view,
                        wgpu::FilterMode::Linear,
                        id,
                    );
                }
                None => {
                    self.texture =
                        Some(renderer.register_native_texture(&render_state.device, &view, wgpu::FilterMode::Linear));
                }
            }
        }
        if let Some(texture) = self.texture {
            let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(texture, rect, whole, egui::Color32::WHITE);
        }
        if frame.again {
            ui.ctx().request_repaint();
        }
        response
    }

    /// Shows `add_contents` at the place the content's marker `id` names, as
    /// GL JS's `Marker` holds an element: its anchor at the marker's point,
    /// moved by its offset, on the map as [`show`](Self::show) last drew it.
    /// Shows nothing while the place is off the map or behind the globe, or
    /// before the map drew. The contents stay upright: egui doesn't turn or
    /// tilt widgets.
    pub fn marker<R>(&self, ctx: &egui::Context, id: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
        let rect = self.rect?;
        let marker = self.component.markers().iter().find(|marker| marker.id == id)?;
        // A view of no size: its top-left corner is where the anchor goes.
        let placement = self.component.place(marker, [0.0, 0.0, 0.0, 0.0])?;
        if placement.hidden || marker.lng_lat.is_none() {
            return None;
        }
        let at = rect.min + egui::vec2(placement.homography[2], placement.homography[5]);
        let [x, y] = marker.anchor.fraction();
        let align = |fraction: f64| match fraction {
            f if f <= 0.0 => egui::Align::Min,
            f if f >= 1.0 => egui::Align::Max,
            _ => egui::Align::Center,
        };
        let area = egui::Area::new(egui::Id::unique(("maplibre-marker", id)))
            .fixed_pos(at)
            .pivot(egui::Align2([align(x), align(y)]))
            .constrain(false);
        Some(area.show(ctx, add_contents).inner)
    }

    /// Hands the map the input of this frame that is its own: what starts over
    /// its area, and what follows from there.
    fn input(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let over = ui.ctx().pointer_hover_pos().filter(|at| rect.contains(*at) && ui.ui_contains_pointer());
        let local = |at: egui::Pos2| ScreenCoordinate::new(f64::from(at.x - rect.min.x), f64::from(at.y - rect.min.y));
        // A point of the map's area that no other layer, such as a popup, covers.
        let mine =
            |at: egui::Pos2| rect.contains(at) && ui.ctx().layer_id_at(at).is_none_or(|layer| layer == ui.layer_id());
        let mut pinched = false;
        for event in ui.input(|input| input.events.clone()) {
            match event {
                // egui also reports a touch as the mouse, which the touches cover.
                egui::Event::PointerButton { .. } | egui::Event::PointerMoved(_) if !self.touches.is_empty() => {}
                egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                    let held = self.buttons.contains(&button);
                    if pressed && mine(pos) && !held {
                        self.buttons.push(button);
                        self.press(0, Phase::Down, local(pos), button, modifiers);
                    } else if !pressed && held {
                        self.buttons.retain(|other| *other != button);
                        self.press(0, Phase::Up, local(pos), button, modifiers);
                    }
                }
                egui::Event::PointerMoved(pos) if !self.buttons.is_empty() => {
                    self.press(0, Phase::Move, local(pos), self.buttons[0], egui::Modifiers::default());
                }
                egui::Event::PointerGone if !self.buttons.is_empty() => {
                    self.buttons.clear();
                    self.press(
                        0,
                        Phase::Cancel,
                        ScreenCoordinate::new(0.0, 0.0),
                        egui::PointerButton::Primary,
                        egui::Modifiers::default(),
                    );
                }
                egui::Event::MouseWheel { unit, delta, modifiers, .. } if over.is_some() => {
                    // The web's sign, which the gestures take: down is positive.
                    let delta = match unit {
                        egui::MouseWheelUnit::Point => WheelDelta::Pixels(-f64::from(delta.y)),
                        egui::MouseWheelUnit::Line => WheelDelta::Notches(-f64::from(delta.y)),
                        egui::MouseWheelUnit::Page => WheelDelta::Notches(-f64::from(delta.y) * 3.0),
                    };
                    let position = over.map_or(ScreenCoordinate::new(0.0, 0.0), local);
                    self.component.gesture(Input::Wheel { position, delta, modifiers: self::modifiers(modifiers) });
                }
                egui::Event::Zoom(scale) if over.is_some() || self.pinching => {
                    pinched = true;
                    self.pinch(over.map(local), f64::from(scale), 0.0);
                }
                egui::Event::Rotate(radians) if over.is_some() || self.pinching => {
                    pinched = true;
                    self.pinch(over.map(local), 1.0, f64::from(radians).to_degrees());
                }
                egui::Event::Touch { id, phase, pos, .. } => self.touch(id.0, phase, local(pos), mine(pos)),
                _ => {}
            }
        }
        if self.pinching && !pinched {
            self.pinching = false;
            let center = over.map_or(ScreenCoordinate::new(0.0, 0.0), local);
            self.component.gesture(Input::Pinch { center, scale: 1.0, rotation: 0.0, phase: PinchPhase::End });
        }
    }

    fn press(
        &mut self,
        id: u32,
        phase: Phase,
        at: ScreenCoordinate,
        button: egui::PointerButton,
        keys: egui::Modifiers,
    ) {
        let button = match button {
            egui::PointerButton::Secondary => Button::Secondary,
            egui::PointerButton::Middle => Button::Middle,
            _ => Button::Primary,
        };
        let press = Press { id, phase, x: at.x, y: at.y, button, touch: false, modifiers: modifiers(keys) };
        self.component.pointer(&press);
    }

    fn pinch(&mut self, center: Option<ScreenCoordinate>, scale: f64, rotation: f64) {
        let phase = if self.pinching { PinchPhase::Update } else { PinchPhase::Begin };
        self.pinching = true;
        let center = center.unwrap_or(ScreenCoordinate::new(0.0, 0.0));
        self.component.gesture(Input::Pinch { center, scale, rotation, phase });
    }

    fn touch(&mut self, touch: u64, phase: egui::TouchPhase, at: ScreenCoordinate, inside: bool) {
        let phase = match phase {
            egui::TouchPhase::Start if inside => Phase::Down,
            egui::TouchPhase::Start => return,
            egui::TouchPhase::Move => Phase::Move,
            egui::TouchPhase::End => Phase::Up,
            egui::TouchPhase::Cancel => Phase::Cancel,
        };
        let id = if phase == Phase::Down {
            let id = self.next_touch;
            self.next_touch = self.next_touch.wrapping_add(1).max(1);
            self.touches.insert(touch, id);
            id
        } else {
            let Some(id) = self.touches.get(&touch).copied() else { return };
            if matches!(phase, Phase::Up | Phase::Cancel) {
                self.touches.remove(&touch);
            }
            id
        };
        let press = Press {
            id,
            phase,
            x: at.x,
            y: at.y,
            button: Button::Primary,
            touch: true,
            modifiers: Modifiers::default(),
        };
        self.component.pointer(&press);
    }
}

fn modifiers(keys: egui::Modifiers) -> Modifiers {
    Modifiers { shift: keys.shift, ctrl: keys.ctrl, alt: keys.alt, meta: keys.mac_cmd }
}
