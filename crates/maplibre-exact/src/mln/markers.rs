//! The canvas's children that the content's `markers` hold at places on the
//! map.
//!
//! Exact lays out each child of the canvas, hands it to the surface on its
//! own and asks after every frame where the surface put it, which is where
//! the child is hit-tested and where accessibility reports it. A child the
//! markers name goes to its place; the others keep their layout and the host
//! draws them, as controls over the map. Where the host hands the surface a
//! child's pixels (Apple), the surface draws the placed children over the
//! map; elsewhere (Android) the host draws them where the placement says.

use exact_gpu::{Placement, wgpu};
use mln_map::MapProjection;
use mln_util::geo::LatLng;

use crate::protocol::Marker;

/// A child of the canvas, as the host last gave it.
struct Child {
    /// Its `testId`.
    name: String,
    /// Where the layout put it in the canvas, in points: x, y, width, height.
    frame: [f32; 4],
    texture: Option<wgpu::TextureView>,
    group: Option<wgpu::BindGroup>,
    placement: Option<Placement>,
}

/// The canvas's children by index.
#[derive(Default)]
pub(super) struct Children {
    children: Vec<Option<Child>>,
    gpu: Option<Gpu>,
}

impl Children {
    /// The host's news of the `index`th child: an empty name and frame with
    /// no texture is a child gone.
    pub(super) fn set(&mut self, index: usize, name: &str, texture: Option<&wgpu::TextureView>, frame: [f32; 4]) {
        if name.is_empty() && texture.is_none() && frame == [0.0; 4] {
            if let Some(slot) = self.children.get_mut(index) {
                *slot = None;
            }
            while self.children.last().is_some_and(Option::is_none) {
                self.children.pop();
            }
            return;
        }
        if self.children.len() <= index {
            self.children.resize_with(index + 1, || None);
        }
        let placement = self.children[index].as_ref().and_then(|child| child.placement);
        self.children[index] =
            Some(Child { name: name.to_owned(), frame, texture: texture.cloned(), group: None, placement });
    }

    pub(super) fn placement(&self, index: usize) -> Option<Placement> {
        self.children.get(index)?.as_ref()?.placement
    }

    /// Places each child a marker names at its marker's point, through
    /// `projection` of the map as drawn, turned by `bearing` and tilted by
    /// `pitch`, on a canvas of `size` points at `scale` pixels a point. A
    /// marker whose child lies off the canvas or behind the globe is hidden,
    /// so it is neither hit nor read out.
    pub(super) fn place(
        &mut self,
        projection: &MapProjection,
        [bearing, pitch]: [f64; 2],
        markers: &[Marker],
        size: [f32; 2],
        scale: f32,
    ) {
        let view = View { bearing, pitch, size, scale };
        for (index, child) in self.children.iter_mut().enumerate() {
            let Some(child) = child else { continue };
            let marker = markers.iter().find(|marker| marker.id == child.name);
            child.placement = marker.and_then(|marker| place(projection, &view, marker, child.frame, index));
        }
    }

    /// Whether a child is placed on the map.
    pub(super) fn any_placed(&self) -> bool {
        self.children.iter().flatten().any(|child| child.placement.is_some())
    }

    /// Draws the placed children the host gave pixels for over what `target`
    /// holds, in their order, the last on top.
    pub(super) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [f32; 2],
    ) {
        let drawn: Vec<usize> = (0..self.children.len())
            .filter(|&index| {
                self.children[index].as_ref().is_some_and(|child| {
                    child.texture.is_some() && child.placement.is_some_and(|placement| !placement.hidden)
                })
            })
            .collect();
        if drawn.is_empty() {
            return;
        }
        let gpu = self.gpu.get_or_insert_with(|| Gpu::new(device));
        let mut vertices = Vec::with_capacity(drawn.len() * 6 * 6);
        for &index in &drawn {
            let child = self.children[index].as_mut().expect("drawn children exist");
            let h = child.placement.expect("drawn children are placed").homography;
            let [width, height] = [child.frame[2], child.frame[3]];
            for [u, v] in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]] {
                let [x, y] = [u * width, v * height];
                let [cx, cy, w] = [0, 3, 6].map(|row| h[row] * x + h[row + 1] * y + h[row + 2]);
                // In clip space before the divide, which interpolates the
                // pixels as the homography maps them.
                vertices.extend_from_slice(&[cx / size[0] * 2.0 - w, w - cy / size[1] * 2.0, 0.0, w, u, v]);
            }
            if child.group.is_none() {
                let view = child.texture.as_ref().expect("drawn children have pixels");
                child.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("maplibre marker"),
                    layout: &gpu.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&gpu.sampler) },
                    ],
                }));
            }
        }
        let bytes: Vec<u8> = vertices.iter().flat_map(|v| v.to_ne_bytes()).collect();
        if gpu.vertices.as_ref().is_none_or(|buffer| buffer.size() < bytes.len() as u64) {
            gpu.vertices = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("maplibre markers"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let pipeline = gpu.pipeline(device, format);
        let pipeline = &gpu.pipelines[pipeline].1;
        let buffer = gpu.vertices.as_ref().expect("made above");
        queue.write_buffer(buffer, 0, &bytes);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("maplibre markers"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, buffer.slice(..));
        for (n, &index) in drawn.iter().enumerate() {
            let child = self.children[index].as_ref().expect("drawn children exist");
            pass.set_bind_group(0, child.group.as_ref(), &[]);
            let first = (n * 6) as u32;
            pass.draw(first..first + 6, 0..1);
        }
    }
}

/// The camera and canvas markers are placed for.
struct View {
    bearing: f64,
    pitch: f64,
    /// The canvas in points.
    size: [f32; 2],
    /// Pixels a point.
    scale: f32,
}

/// Where `marker` puts a child laid out at `frame`, the `index`th of the
/// canvas's, as GL JS's `Marker` places its element: the anchor at the
/// marker's point plus the offset, or where the layout put it, turned by the
/// rotation, less the bearing when it turns with the map, and squashed as the
/// map pitches when it lies on the map. An upright child sits on whole pixels,
/// so that its pixels stay sharp. `None` leaves the child to the layout.
fn place(projection: &MapProjection, view: &View, marker: &Marker, frame: [f32; 4], index: usize) -> Option<Placement> {
    let [width, height] = [f64::from(frame[2]), f64::from(frame[3])];
    let [ax, ay] = marker.anchor.fraction();
    let [ax, ay] = [ax * width, ay * height];
    let angle = (marker.rotation - if marker.turns_with_map() { view.bearing } else { 0.0 }).to_radians();
    let squash = if marker.lies_on_map() { view.pitch.to_radians().cos() } else { 1.0 };
    let upright = angle.rem_euclid(std::f64::consts::TAU) == 0.0 && squash == 1.0;
    let point = match marker.lng_lat {
        Some([lng, lat]) => LatLng::new(lat, lng)
            .ok()
            .filter(|at| !projection.is_location_occluded(at))
            .map(|at| projection.pixel_for_lat_lng(&at))
            .filter(|point| point.x.is_finite() && point.y.is_finite())
            .map(|point| [point.x, point.y]),
        None if upright && marker.offset == [0.0, 0.0] => return None,
        None => Some([f64::from(frame[0]) + ax, f64::from(frame[1]) + ay]),
    };
    let Some([x, y]) = point.map(|[x, y]| [x + marker.offset[0], y + marker.offset[1]]) else {
        return Some(Placement { hidden: true, ..at(1.0, 0.0, 0.0, 1.0, 0.0, 0.0, index) });
    };
    // The child turned about its anchor, squashed, then moved to the point.
    let (sin, cos) = angle.sin_cos();
    let [a, b, c, d] = [cos, -sin, squash * sin, squash * cos];
    let [mut tx, mut ty] = [x - (a * ax + b * ay), y - (c * ax + d * ay)];
    if upright {
        let snap = |n: f64| (n * f64::from(view.scale)).round() / f64::from(view.scale);
        [tx, ty] = [snap(tx), snap(ty)];
    }
    let mut placement = at(a, b, c, d, tx, ty, index);
    let corners = [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]]
        .map(|[u, v]| [a * u + b * v + tx, c * u + d * v + ty]);
    let [x0, y0, x1, y1] = corners.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |[x0, y0, x1, y1], [x, y]| {
        [x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y)]
    });
    placement.hidden = x1 < 0.0 || y1 < 0.0 || x0 > f64::from(view.size[0]) || y0 > f64::from(view.size[1]);
    Some(placement)
}

/// The placement of the affine map `[a b tx; c d ty]` for the `index`th child,
/// later children nearer.
fn at(a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64, index: usize) -> Placement {
    Placement {
        hidden: false,
        homography: [a, b, tx, c, d, ty, 0.0, 0.0, 1.0].map(|n| n as f32),
        depth: index as f32 + 1.0,
        clip_depth: [[0.0, 0.0, 1.0]; 2],
    }
}

/// The pipeline that draws a child's pixels, made at the first marker drawn.
struct Gpu {
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    vertices: Option<wgpu::Buffer>,
}

impl Gpu {
    fn new(device: &wgpu::Device) -> Gpu {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("maplibre marker"),
            source: wgpu::ShaderSource::Wgsl(include_str!("marker.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("maplibre marker"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("maplibre marker"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("maplibre marker"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Gpu { module, layout, pipeline_layout, sampler, pipelines: Vec::new(), vertices: None }
    }

    /// The index of the pipeline for `format`, made the first time.
    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> usize {
        match self.pipelines.iter().position(|(f, _)| *f == format) {
            Some(index) => index,
            None => {
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("maplibre marker"),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.module,
                        entry_point: Some("vs"),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: 24,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2],
                        })],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.module,
                        entry_point: Some(if format.is_srgb() { "fs_linear" } else { "fs" }),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                });
                self.pipelines.push((format, pipeline));
                self.pipelines.len() - 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mln_map::{CameraOptions, Map, MapOptions};
    use mln_util::size::Size;

    use super::*;
    use crate::protocol::{Alignment, Anchor};

    /// A 400×300 map looking straight down at 0, 0 at zoom 2, turned and
    /// pitched as given.
    fn map(bearing: f64, pitch: f64) -> (MapProjection, View) {
        let options = MapOptions { size: Size::new(400, 300), pixel_ratio: 1.0, ..MapOptions::default() };
        let mut map = Map::new(options).unwrap();
        map.jump_to(&CameraOptions {
            center: LatLng::new(0.0, 0.0).ok(),
            zoom: Some(2.0),
            bearing: Some(bearing),
            pitch: Some(pitch),
            ..CameraOptions::default()
        });
        (MapProjection::new(&map), View { bearing, pitch, size: [400.0, 300.0], scale: 2.0 })
    }

    fn marker(lng_lat: Option<[f64; 2]>) -> Marker {
        Marker {
            id: "m".into(),
            lng_lat,
            anchor: Anchor::Center,
            offset: [0.0, 0.0],
            rotation: 0.0,
            rotation_alignment: Alignment::Auto,
            pitch_alignment: Alignment::Auto,
        }
    }

    fn near(a: [f32; 9], b: [f32; 9]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    #[test]
    fn a_marker_puts_its_anchor_at_its_place_plus_the_offset() {
        let (map, view) = map(0.0, 0.0);
        let callout = Marker { anchor: Anchor::Bottom, offset: [0.0, -4.0], ..marker(Some([0.0, 0.0])) };
        let placement = place(&map, &view, &callout, [0.0, 0.0, 80.0, 40.0], 0).unwrap();
        assert!(!placement.hidden);
        assert!(near(placement.homography, [1.0, 0.0, 160.0, 0.0, 1.0, 106.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_marker_off_the_canvas_is_hidden() {
        let (map, view) = map(0.0, 0.0);
        let placement = place(&map, &view, &marker(Some([120.0, 0.0])), [0.0, 0.0, 80.0, 40.0], 0).unwrap();
        assert!(placement.hidden);
    }

    #[test]
    fn an_upright_marker_without_a_place_is_left_to_the_layout() {
        let (map, view) = map(0.0, 0.0);
        let compass = Marker { rotation_alignment: Alignment::Map, ..marker(None) };
        assert_eq!(place(&map, &view, &compass, [354.0, 10.0, 36.0, 36.0], 0), None);
    }

    #[test]
    fn a_compass_turns_against_the_bearing_about_its_center_and_lies_on_the_map() {
        let (map, view) = map(90.0, 60.0);
        let compass = Marker { rotation_alignment: Alignment::Map, ..marker(None) };
        let placement = place(&map, &view, &compass, [354.0, 10.0, 36.0, 36.0], 0).unwrap();
        // Turned a quarter counterclockwise and squashed to half its height
        // (cos 60°), its center where the layout put it.
        let h = placement.homography;
        assert!(near(h, [0.0, 1.0, h[2], -0.5, 0.0, h[5], 0.0, 0.0, 1.0]), "{h:?}");
        let center = [h[0] * 18.0 + h[1] * 18.0 + h[2], h[3] * 18.0 + h[4] * 18.0 + h[5]];
        assert!((center[0] - 372.0).abs() < 1e-3 && (center[1] - 28.0).abs() < 1e-3, "{center:?}");
    }
}
