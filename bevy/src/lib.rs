//! MapLibre maps in [Bevy](https://bevyengine.org) apps, drawn by mln-rs on
//! Bevy's wgpu device through maplibre-rust-ui's engine.
//!
//! An app adds [`MapLibrePlugin`] and puts a [`MapLibreMap`] on a UI node,
//! with the map it declares ([`model::Inputs`]). It sends the map commands
//! as [`MapCommand`] messages and hears of its events as [`MapEvent`]
//! messages, each with the map's record. [`MapPointer`] messages hand a map
//! input of the app's own:
//!
//! ```ignore
//! App::new()
//!     .add_plugins((DefaultPlugins, MapLibrePlugin))
//!     .add_systems(Startup, |mut commands: Commands| {
//!         commands.spawn(Camera2d);
//!         commands.spawn((MapLibreMap::new(inputs), Node { width: percent(100), height: percent(100), ..default() }));
//!     });
//! ```
//!
//! The map draws into a texture of its own on Bevy's device, in the main
//! world, and the node shows it as its image. It asks for frames while it
//! loads, animates or follows a gesture, and takes the mouse, a trackpad's
//! scroll, pinch and rotation, and touches over its node.

use std::collections::HashMap;

use bevy::app::{App, Plugin, PostUpdate};
use bevy::asset::{AssetEvent, AssetId, Assets, Handle, RenderAssetUsages};
use bevy::camera::visibility::Visibility;
use bevy::ecs::prelude::*;
use bevy::image::Image;
use bevy::input::ButtonInput;
use bevy::input::gestures::{PinchGesture, RotationGesture};
use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::math::Vec2;
use bevy::render::render_asset::{RenderAssets, prepare_assets};
use bevy::render::render_resource::{Sampler, Texture, TextureView};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use bevy::sprite_render::SpriteAssetEvents;
use bevy::ui::widget::{ImageNode, NodeImageMode};
use bevy::ui::{ComputedNode, Node, PositionType, UiGlobalTransform, UiSystems, Val};
use bevy::window::{PrimaryWindow, RequestRedraw, Window};
pub use maplibre_rust_ui::Phase;
use maplibre_rust_ui::component::Component as Map;
pub use maplibre_rust_ui::component::Press;
pub use maplibre_rust_ui::gestures::Button;
use maplibre_rust_ui::gestures::{Input, Modifiers, PinchPhase, WheelDelta};
pub use maplibre_rust_ui::model;
use maplibre_rust_ui::model::{Command, Event, Inputs, Record};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::size::Size;
use maplibre_rust_ui::{Viewport, wgpu};

/// Draws the [`MapLibreMap`]s of the app's UI.
pub struct MapLibrePlugin;

impl Plugin for MapLibrePlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send(Maps::default())
            .init_resource::<MapTextures>()
            .add_message::<MapCommand>()
            .add_message::<MapEvent>()
            .add_message::<MapPointer>()
            // Before the layout, which then lays out the markers where the
            // frame drew their places, and after it the map's size of the
            // frame before, which a resize changes a frame late.
            .add_systems(PostUpdate, (add_maps, run_maps, place_markers, drop_maps).chain().before(UiSystems::Layout));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.init_resource::<RenderedMaps>().add_systems(ExtractSchedule, extract_map_textures).add_systems(
                Render,
                show_map_textures.in_set(RenderSystems::PrepareAssets).after(prepare_assets::<GpuImage>),
            );
        }
    }
}

/// A map on a UI node, as the app declares it now. The map takes what
/// changed at its next frame: a camera moves only when its value changes.
#[derive(Component, Clone, Debug)]
#[require(ImageNode)]
pub struct MapLibreMap {
    pub inputs: Inputs,
}

impl MapLibreMap {
    #[must_use]
    pub fn new(inputs: Inputs) -> MapLibreMap {
        MapLibreMap { inputs }
    }
}

/// A UI node, a child of a [`MapLibreMap`]'s node, that the content's marker
/// of the same `id` holds at its place on the map, as GL JS's `Marker` holds
/// an element: its anchor at the marker's point, moved by its offset. It is
/// hidden while its place is off the map or behind the globe. The plugin
/// positions it, so it has `position_type: Absolute`.
#[derive(Component, Clone, Debug)]
pub struct MapMarker {
    pub id: String,
}

/// A command for the map on the node `map`, which starts at its next frame.
#[derive(Message, Clone, Debug)]
pub struct MapCommand {
    pub map: Entity,
    pub command: Command,
}

/// A pointer on the map on the node `map`, in logical pixels from the
/// node's top-left corner, which the map takes as it takes the mouse and
/// touches: input of the app's own, such as from a test or a remote.
#[derive(Message, Clone, Copy, Debug)]
pub struct MapPointer {
    pub map: Entity,
    pub press: Press,
}

/// An event of the map on the node `map`, of those its inputs listen for,
/// with the map's record.
#[derive(Message, Clone, Debug)]
pub struct MapEvent {
    pub map: Entity,
    pub event: Event,
    pub record: Option<Record>,
}

/// The maps, by their nodes, and the input each follows. A map's session
/// renders on the thread that drives it, so they stay on the main thread.
#[derive(Default)]
struct Maps {
    maps: HashMap<Entity, MapState>,
}

struct MapState {
    map: Map,
    image: Handle<Image>,
    /// The mouse buttons pressed on the map, which it follows until they lift.
    buttons: Vec<MouseButton>,
    /// The touches on the map, by Bevy's ids, with the map's.
    touches: HashMap<u64, u32>,
    next_touch: u32,
    /// Whether a trackpad pinch or rotation goes on.
    pinching: bool,
}

/// The texture each map draws into, by its node's image, for the render
/// world to show.
#[derive(Resource, Default, Clone)]
struct MapTextures {
    textures: HashMap<AssetId<Image>, wgpu::Texture>,
}

/// Gives each new map its state and its node an image of its own.
fn add_maps(
    mut maps: NonSendMut<Maps>,
    mut images: ResMut<Assets<Image>>,
    mut nodes: Query<(Entity, &MapLibreMap, &mut ImageNode), Added<MapLibreMap>>,
) {
    for (entity, map, mut node) in &mut nodes {
        let image = images.add(Image::new_uninit(
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            wgpu::TextureDimension::D2,
            wgpu::TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        ));
        node.image = image.clone();
        // The layout alone sizes the map: an image's own size would hold the
        // node to its placeholder's square.
        node.image_mode = NodeImageMode::Stretch;
        maps.maps.insert(
            entity,
            MapState {
                map: Map::with_inputs(map.inputs.clone()),
                image,
                buttons: Vec::new(),
                touches: HashMap::new(),
                next_touch: 1,
                pinching: false,
            },
        );
    }
}

/// Lets go of the maps whose nodes went.
fn drop_maps(
    mut maps: NonSendMut<Maps>,
    mut removed: RemovedComponents<MapLibreMap>,
    mut textures: ResMut<MapTextures>,
) {
    for entity in removed.read() {
        if let Some(state) = maps.maps.remove(&entity) {
            textures.textures.remove(&state.image.id());
        }
    }
}

/// The input of this frame, which each map takes what is its own of.
#[derive(bevy::ecs::system::SystemParam)]
struct FrameInput<'w, 's> {
    buttons: MessageReader<'w, 's, MouseButtonInput>,
    wheels: MessageReader<'w, 's, MouseWheel>,
    pinches: MessageReader<'w, 's, PinchGesture>,
    rotations: MessageReader<'w, 's, RotationGesture>,
    touches: MessageReader<'w, 's, TouchInput>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    window: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

/// Runs a frame of each map on Bevy's device: its inputs, commands and input,
/// then the map, whose texture the node shows.
#[allow(clippy::too_many_arguments)]
fn run_maps(
    mut maps: NonSendMut<Maps>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    nodes: Query<(Entity, Ref<MapLibreMap>, &ComputedNode, &UiGlobalTransform)>,
    mut input: FrameInput,
    mut commands: MessageReader<MapCommand>,
    mut pointers: MessageReader<MapPointer>,
    mut events: MessageWriter<MapEvent>,
    mut textures: ResMut<MapTextures>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let window = input.window.single().ok();
    let cursor = window.and_then(Window::physical_cursor_position);
    let keys = &input.keys;
    let modifiers = Modifiers {
        shift: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        ctrl: keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
        alt: keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
        meta: keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight]),
    };
    let buttons: Vec<MouseButtonInput> = input.buttons.read().cloned().collect();
    let wheels: Vec<MouseWheel> = input.wheels.read().cloned().collect();
    let pinches: Vec<f32> = input.pinches.read().map(|pinch| pinch.0).collect();
    let rotations: Vec<f32> = input.rotations.read().map(|rotation| rotation.0).collect();
    let touches: Vec<TouchInput> = input.touches.read().cloned().collect();
    let sent: Vec<MapCommand> = commands.read().cloned().collect();
    let pressed: Vec<MapPointer> = pointers.read().copied().collect();
    let scale = window.map_or(1.0, Window::scale_factor);
    let mut again = false;
    for (entity, declared, node, transform) in &nodes {
        let Some(state) = maps.maps.get_mut(&entity) else { continue };
        if declared.is_changed() {
            state.map.set_inputs(declared.inputs.clone());
        }
        for command in sent.iter().filter(|command| command.map == entity) {
            state.map.send(command.command.clone());
        }
        for pointer in pressed.iter().filter(|pointer| pointer.map == entity) {
            state.map.pointer(&pointer.press);
        }
        // Points on the node, in logical pixels from its top-left corner.
        let local = |physical: Vec2| {
            node.normalize_point(*transform, physical).map(|at| {
                let at = (at + Vec2::splat(0.5)) * node.size() * node.inverse_scale_factor();
                ScreenCoordinate::new(f64::from(at.x), f64::from(at.y))
            })
        };
        let over = cursor.filter(|at| node.contains_point(*transform, *at)).and_then(local);
        for button in &buttons {
            let held = state.buttons.contains(&button.button);
            let pressed = button.state.is_pressed();
            let Some(at) = cursor.and_then(local) else { continue };
            if pressed && over.is_some() && !held {
                state.buttons.push(button.button);
                state.press(0, Phase::Down, at, button.button, modifiers);
            } else if !pressed && held {
                state.buttons.retain(|other| *other != button.button);
                state.press(0, Phase::Up, at, button.button, modifiers);
            }
        }
        if let (Some(first), Some(at)) = (state.buttons.first().copied(), cursor.and_then(local)) {
            state.press(0, Phase::Move, at, first, modifiers);
        }
        if let Some(at) = over {
            for wheel in &wheels {
                // The web's sign, which the gestures take: down is positive.
                let delta = match wheel.unit {
                    MouseScrollUnit::Pixel => WheelDelta::Pixels(-f64::from(wheel.y)),
                    MouseScrollUnit::Line => WheelDelta::Notches(-f64::from(wheel.y)),
                };
                state.map.gesture(Input::Wheel { position: at, delta, modifiers });
            }
        }
        let pinch_at = over.or(state.pinching.then(|| ScreenCoordinate::new(0.0, 0.0)));
        let pinched = pinch_at.is_some() && (!pinches.is_empty() || !rotations.is_empty());
        if let Some(at) = pinch_at.filter(|_| pinched) {
            for magnification in &pinches {
                state.pinch(at, 1.0 + f64::from(*magnification), 0.0);
            }
            // Bevy turns counterclockwise, the gestures clockwise.
            for degrees in &rotations {
                state.pinch(at, 1.0, -f64::from(*degrees));
            }
        }
        if state.pinching && !pinched {
            state.pinching = false;
            let center = over.unwrap_or(ScreenCoordinate::new(0.0, 0.0));
            state.map.gesture(Input::Pinch { center, scale: 1.0, rotation: 0.0, phase: PinchPhase::End });
        }
        for touch in &touches {
            // Touches come in logical pixels.
            let physical = touch.position * scale;
            let inside = node.contains_point(*transform, physical);
            if let Some(at) = local(physical) {
                state.touch(touch.id, touch.phase, at, inside);
            }
        }
        let size = node.size() * node.inverse_scale_factor();
        // A node not laid out yet has no size to make the map at.
        if size.min_element() < 1.0 {
            continue;
        }
        let viewport = Viewport {
            size: Size::new(size.x.round().max(1.0) as u32, size.y.round().max(1.0) as u32),
            scale: 1.0 / node.inverse_scale_factor(),
        };
        let frame = state.map.frame_texture(device.wgpu_device(), &queue, viewport);
        if let Some(texture) = frame.texture {
            textures.textures.insert(state.image.id(), texture);
        }
        let record = state.map.record();
        for post in frame.posts {
            events.write(MapEvent { map: entity, event: post.event, record: record.clone() });
        }
        again |= frame.again;
    }
    if again {
        redraw.write(RequestRedraw);
    }
}

/// Moves each marker to where its map's last frame drew its place.
fn place_markers(
    maps: NonSend<Maps>,
    mut markers: Query<(&MapMarker, &ChildOf, &mut Node, &ComputedNode, &mut Visibility)>,
) {
    for (marker, parent, mut node, computed, mut visibility) in &mut markers {
        let Some(state) = maps.maps.get(&parent.parent()) else { continue };
        let Some(declared) = state.map.markers().iter().find(|declared| declared.id == marker.id) else { continue };
        let size = computed.size() * computed.inverse_scale_factor();
        let Some(placement) = state.map.place(declared, [0.0, 0.0, size.x, size.y]) else { continue };
        let shown = if placement.hidden { Visibility::Hidden } else { Visibility::Inherited };
        visibility.set_if_neq(shown);
        node.position_type = PositionType::Absolute;
        node.left = Val::Px(placement.homography[2]);
        node.top = Val::Px(placement.homography[5]);
    }
}

impl MapState {
    fn press(&mut self, id: u32, phase: Phase, at: ScreenCoordinate, button: MouseButton, modifiers: Modifiers) {
        let button = match button {
            MouseButton::Right => Button::Secondary,
            MouseButton::Middle => Button::Middle,
            _ => Button::Primary,
        };
        self.map.pointer(&Press { id, phase, x: at.x, y: at.y, button, touch: false, modifiers });
    }

    fn pinch(&mut self, center: ScreenCoordinate, scale: f64, rotation: f64) {
        let phase = if self.pinching { PinchPhase::Update } else { PinchPhase::Begin };
        self.pinching = true;
        self.map.gesture(Input::Pinch { center, scale, rotation, phase });
    }

    fn touch(&mut self, touch: u64, phase: TouchPhase, at: ScreenCoordinate, inside: bool) {
        let phase = match phase {
            TouchPhase::Started if inside => Phase::Down,
            TouchPhase::Started => return,
            TouchPhase::Moved => Phase::Move,
            TouchPhase::Ended => Phase::Up,
            TouchPhase::Canceled => Phase::Cancel,
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
        self.map.pointer(&press);
    }
}

/// The maps' textures in the render world, with what showing each takes.
#[derive(Resource, Default)]
struct RenderedMaps {
    textures: HashMap<AssetId<Image>, wgpu::Texture>,
    shown: HashMap<AssetId<Image>, GpuImage>,
}

fn extract_map_textures(textures: Extract<Res<MapTextures>>, mut rendered: ResMut<RenderedMaps>) {
    rendered.textures.clone_from(&textures.textures);
}

/// Puts each map's texture in place of its node's image, after Bevy prepared
/// the images of the frame. A new texture counts as a change of the image, so
/// that the UI drops what it keeps of the old one.
fn show_map_textures(
    mut rendered: ResMut<RenderedMaps>,
    mut images: ResMut<RenderAssets<GpuImage>>,
    mut changed: Option<ResMut<SpriteAssetEvents>>,
    device: Res<RenderDevice>,
) {
    let RenderedMaps { textures, shown } = &mut *rendered;
    shown.retain(|id, _| textures.contains_key(id));
    for (id, texture) in textures.iter() {
        let current = shown.get(id).is_some_and(|image| *image.texture == *texture);
        if !current {
            shown.insert(*id, gpu_image(&device, texture));
            if let Some(changed) = changed.as_mut() {
                changed.images.push(AssetEvent::Modified { id: *id });
            }
        }
        if let Some(image) = shown.get(id) {
            images.insert(*id, image.clone());
        }
    }
}

/// The map's texture as Bevy's image, sampled through an sRGB view, since
/// Bevy samples colors as linear.
fn gpu_image(device: &RenderDevice, texture: &wgpu::Texture) -> GpuImage {
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        ..Default::default()
    });
    let sampler = device.wgpu_device().create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    GpuImage {
        texture: Texture::from(texture.clone()),
        texture_view: TextureView::from(view),
        sampler: Sampler::from(sampler),
        texture_descriptor: wgpu::TextureDescriptor {
            label: None,
            size: texture.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: texture.usage(),
            view_formats: &[],
        },
        texture_view_descriptor: None,
        had_data: true,
    }
}
