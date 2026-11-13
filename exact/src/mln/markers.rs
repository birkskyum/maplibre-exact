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
use maplibre_rust_ui::placement::{self, View};

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

    /// Places each child a marker names, as maplibre-rust-ui's placement
    /// finds it for the drawn view.
    pub(super) fn place(&mut self, view: &View, markers: &[Marker]) {
        for (index, child) in self.children.iter_mut().enumerate() {
            let Some(child) = child else { continue };
            let marker = markers.iter().find(|marker| marker.id == child.name);
            child.placement = marker.and_then(|marker| placement::place(view, marker, child.frame)).map(|placed| {
                Placement {
                    hidden: placed.hidden,
                    homography: placed.homography,
                    // Later children nearer.
                    depth: index as f32 + 1.0,
                    clip_depth: [[0.0, 0.0, 1.0]; 2],
                }
            });
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
