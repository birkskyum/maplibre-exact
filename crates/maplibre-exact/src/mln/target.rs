//! The texture mln-rs draws the map into: the surface's own, on Exact's
//! device, which the blit then samples. mln-rs submits each frame to the
//! device's queue before the surface's draw goes to it, so the queue orders
//! the blit after the frame, and the next frame after the blit.

use std::rc::Rc;
use std::sync::Arc;

use exact_gpu::wgpu;

use mln_gfx::SyncKind;
use mln_renderer::{Renderer, UpdateParameters};
use mln_util::size::Size;
use mln_webgpu::{BorrowedTarget, BorrowedTextures, RendererBackend};

/// Premultiplied, as mln-rs draws.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub(super) struct MapTarget {
    texture: wgpu::Texture,
    target: BorrowedTarget,
    /// Whether the texture holds a frame of its size.
    drawn: bool,
}

impl MapTarget {
    /// A target of `size` pixels, or `None` for an empty one, which has no
    /// frames.
    pub(super) fn new(backend: &Rc<RendererBackend>, size: Size) -> Result<Option<MapTarget>, String> {
        let Some(textures) = textures(backend, size) else { return Ok(None) };
        let texture = textures.textures[0].clone();
        let target = BorrowedTarget::new(backend.clone(), textures, SyncKind::QueueOrder)
            .map_err(|e| format!("maplibre: {}", e.message))?;
        Ok(Some(MapTarget { texture, target, drawn: false }))
    }

    /// Draws the next frames at `size`, from the next one on.
    pub(super) fn resize(
        this: &mut Option<MapTarget>,
        backend: &Rc<RendererBackend>,
        size: Size,
    ) -> Result<(), String> {
        let Some(target) = this.as_mut() else {
            *this = MapTarget::new(backend, size)?;
            return Ok(());
        };
        if size == target.target_size() {
            return Ok(());
        }
        let Some(textures) = textures(backend, size) else {
            *this = None;
            return Ok(());
        };
        let texture = textures.textures[0].clone();
        target.target.replace(textures).map_err(|e| format!("maplibre: {}", e.message))?;
        target.texture = texture;
        target.drawn = false;
        Ok(())
    }

    /// Renders `parameters`, if the renderer has a frame for them.
    pub(super) fn render(&mut self, renderer: &mut Renderer, parameters: &Arc<UpdateParameters>) {
        if renderer.render(parameters, &self.target).is_some() {
            self.drawn = true;
        }
    }

    /// The texture, once it holds a frame.
    pub(super) fn frame(&self) -> Option<&wgpu::Texture> {
        self.drawn.then_some(&self.texture)
    }

    fn target_size(&self) -> Size {
        Size::new(self.texture.width(), self.texture.height())
    }
}

/// A one-texture ring of `size` pixels, or `None` for an empty size.
fn textures(backend: &Rc<RendererBackend>, size: Size) -> Option<BorrowedTextures> {
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let texture = backend.device().device().create_texture(&wgpu::TextureDescriptor {
        label: Some("maplibre map"),
        size: wgpu::Extent3d { width: size.width, height: size.height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    Some(BorrowedTextures { textures: vec![texture], size, depth_textures: Vec::new() })
}
