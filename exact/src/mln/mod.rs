//! The map drawn by mln-rs on Exact's GPU canvas.
//!
//! Exact calls a surface on one thread. The map is maplibre-rust-ui's
//! [`Engine`], whose render session renders on the surface's thread while the
//! surface services it, into a texture of the surface's, on Exact's device.
//! The surface then draws the texture into Exact's target with the encoder it
//! is given, which Exact submits after. The surface asks for frames while the
//! map loads, animates or follows a gesture, and for none once it is idle.

mod blit;
mod input;
mod markers;

use std::collections::HashMap;

use exact_gpu::{
    AssetChanges, AssetError, ChildrenMode, Frame, InputEvent, Placement, Surface, SurfaceError, Value,
    wgpu,
};
use maplibre_rust_ui::map::MapProjection;
use maplibre_rust_ui::placement::View;
use maplibre_rust_ui::runtime::{Device, Driver, RenderTarget};
use maplibre_rust_ui::texture::MapTexture;
use maplibre_rust_ui::util::image::PremultipliedImage;
use maplibre_rust_ui::util::size::Size;
use maplibre_rust_ui::{Engine, Viewport};

use crate::protocol::{self, Command, Event, Inputs, Record};

/// The factory a GPU module's registry names.
#[must_use]
pub fn surface() -> Box<dyn Surface> {
    Box::new(MapSurface::default())
}

/// A map on a canvas.
#[derive(Default)]
pub struct MapSurface {
    inputs: Option<Inputs>,
    engine: Option<Engine>,
    /// The texture the engine renders into, on Exact's device.
    texture: Option<MapTexture>,
    blit: Option<blit::Blit>,
    /// Commands posted before the map was made.
    pending: Vec<Command>,
    events: Vec<String>,
    published: Option<Record>,
    record: Option<Record>,
    error: Option<SurfaceError>,
    input_changed: bool,
    /// The content's images asked of the host, by path, and those decoded.
    asked: Vec<String>,
    decoded: HashMap<String, PremultipliedImage>,
    /// The canvas's children, some of which the content places on the map.
    children: markers::Children,
}

impl MapSurface {
    /// Posts `event` to the canvas's `message=` action if the app listens for it.
    fn post(&mut self, event: Event) {
        if self.inputs.as_ref().is_some_and(|inputs| inputs.events.contains(&event)) {
            self.events.push(event.name().to_owned());
        }
    }
}

impl Surface for MapSurface {
    fn arguments(&self) -> Vec<(&'static str, Value)> {
        protocol::argument_defaults()
    }

    fn bind(&mut self, inputs: &[Value], _at_ms: Option<f64>) -> Result<(), SurfaceError> {
        self.inputs = Some(protocol::inputs(inputs).map_err(SurfaceError)?);
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
        let viewport = viewport(frame);
        let pixels = pixels(frame);
        if self.engine.is_none() {
            let texture = MapTexture::new(device, pixels);
            let shared = Device::WebGpu(maplibre_rust_ui::webgpu::Device::from_wgpu(
                None,
                device.clone(),
                queue.clone(),
                None,
            ));
            let target = RenderTarget::Borrowed { size: viewport.size, ring: texture.ring(), device: shared };
            match Engine::new(viewport, frame.now_ms, inputs.clone(), target, Driver::CallerThread) {
                Ok(engine) => {
                    self.engine = Some(engine);
                    self.texture = Some(texture);
                }
                Err(message) => {
                    self.post(Event::Error);
                    self.error = Some(SurfaceError(message));
                    return false;
                }
            }
        }
        let (Some(engine), Some(texture)) = (self.engine.as_mut(), self.texture.as_mut()) else { return false };
        // A canvas of a new size gets a texture of its size, which resizes the map.
        if viewport != engine.viewport() || pixels != texture.pixels() {
            let resized = MapTexture::new(device, pixels);
            match engine.set_ring(viewport, resized.ring()) {
                Ok(()) => *texture = resized,
                Err(message) => self.error = Some(SurfaceError(message)),
            }
        }
        engine.apply_inputs(&inputs);
        for command in std::mem::take(&mut self.pending) {
            engine.command(command, frame.now_ms);
        }
        let mut events = Vec::new();
        let wants_frame = engine.frame(frame.now_ms, &self.decoded, &mut events);
        let size = [frame.width, frame.height];
        if engine.has_frame() {
            if let Some(drawn) = engine.drawn_view()
                && let Ok(projection) = MapProjection::from_view(drawn)
            {
                let view = View {
                    projection: &projection,
                    bearing: drawn.camera.bearing,
                    pitch: drawn.camera.pitch,
                    size,
                    scale: frame.scale,
                };
                self.children.place(&view, &inputs.content.markers);
            }
            let blit = self.blit.get_or_insert_with(|| blit::Blit::new(device));
            blit.draw(device, encoder, target, format, &texture.texture);
            self.children.draw(device, queue, encoder, target, format, size);
        }
        if let Some(refusal) = engine.take_refusal() {
            self.error = Some(SurfaceError(refusal));
        }
        self.record = Some(engine.record());
        for event in events {
            self.post(event);
        }
        wants_frame
    }

    /// Asks the host for the content's images. Content names them from the
    /// app's folder, as Contract does (`assets/pin.png`), and the host takes
    /// names inside `assets/`.
    fn assets(&mut self) -> AssetChanges {
        let wanted: Vec<String> =
            self.inputs.iter().flat_map(|inputs| inputs.content.images.iter().map(|image| image.src.clone())).collect();
        let retired: Vec<String> =
            self.asked.iter().filter(|src| !wanted.contains(src)).filter_map(|src| in_assets(src)).collect();
        self.asked.retain(|src| wanted.contains(src));
        self.decoded.retain(|src, _| wanted.contains(src));
        let mut requests = Vec::new();
        for src in wanted {
            if self.asked.contains(&src) {
                continue;
            }
            self.asked.push(src.clone());
            match in_assets(&src) {
                Some(name) => requests.push(name),
                None => {
                    let message = format!("maplibre: image {src} must be a path under assets/");
                    self.post(Event::Error);
                    self.error = Some(SurfaceError(message));
                }
            }
        }
        AssetChanges { requests, retired }
    }

    fn asset(&mut self, name: &str, bytes: Result<&[u8], AssetError>) {
        let name = &format!("assets/{name}");
        let decoded = match bytes {
            Ok(bytes) => maplibre_rust_ui::image::decode_image(bytes).map_err(|e| e.message),
            Err(AssetError::Missing) => Err("no such asset".to_owned()),
            Err(AssetError::Failed(why)) => Err(why),
        };
        match decoded {
            Ok(image) => {
                self.decoded.insert(name.to_owned(), image);
                if let Some(engine) = self.engine.as_mut() {
                    engine.images_changed();
                }
            }
            Err(why) => {
                let message = format!("maplibre: image {name}: {why}");
                if let Some(engine) = self.engine.as_mut() {
                    engine.report(message.clone());
                }
                self.post(Event::Error);
                self.error = Some(SurfaceError(message));
            }
        }
    }

    fn frame_ready(&self) -> bool {
        self.engine.as_ref().is_some_and(Engine::has_frame)
    }

    fn device_lost(&mut self) {
        self.engine = None;
        self.texture = None;
        self.blit = None;
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
                        engine.report(message.clone());
                    }
                    self.post(Event::Error);
                    self.error = Some(SurfaceError(message));
                }
            }
            self.input_changed = true;
            return;
        }
        let Some(engine) = self.engine.as_mut() else { return };
        if let InputEvent::Blur { .. } = event {
            engine.blur();
        }
        if let Some(pointer) = input::pointer(event) {
            self.input_changed = engine.pointer(&pointer);
        }
        if engine.interactive()
            && let Some((input, at_ms)) = input::translate(event)
        {
            self.input_changed |= engine.gesture(input, at_ms) || engine.pressed();
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

    /// Each child on its own once the content places some on the map, and
    /// the host's ordinary overlay otherwise, which costs it less.
    fn children_mode(&self) -> ChildrenMode {
        let placing = self.inputs.as_ref().is_some_and(|inputs| !inputs.content.markers.is_empty());
        if placing || self.children.any_placed() { ChildrenMode::Each } else { ChildrenMode::Overlay }
    }

    fn child(&mut self, index: usize, name: &str, texture: Option<&wgpu::TextureView>, frame: [f32; 4]) {
        self.children.set(index, name, texture, frame);
    }

    fn placement(&self, index: usize) -> Option<Placement> {
        self.children.placement(index)
    }
}

/// The name the host takes for an app-relative path under `assets/`.
fn in_assets(src: &str) -> Option<String> {
    src.strip_prefix("assets/").filter(|name| !name.is_empty()).map(str::to_owned)
}

/// The canvas in whole points, at least one by one, and its scale.
fn viewport(frame: &Frame) -> Viewport {
    let size = Size::new((frame.width.round() as u32).max(1), (frame.height.round() as u32).max(1));
    Viewport { size, scale: frame.scale }
}

/// The canvas's size in pixels, at least one by one.
fn pixels(frame: &Frame) -> Size {
    let (width, height) = frame.pixels();
    Size::new(width.max(1), height.max(1))
}
