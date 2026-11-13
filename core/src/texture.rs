//! The texture the map renders into: the surface's own, on Exact's device,
//! lent to the render session as a ring of one, which the blit then samples.
//! The session renders only while the surface services it, before the
//! surface draws, and Exact submits that draw after the session's frame, so
//! the queue orders the blit after the frame and the next frame after the
//! blit.

use exact_gpu::wgpu;
use maplibre_rust_ui::runtime::BorrowedRing;
use maplibre_rust_ui::util::size::Size;
use maplibre_rust_ui::webgpu::BorrowedTextures;

/// Premultiplied, as mln-rs draws.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub(super) struct MapTexture {
    pub(super) texture: wgpu::Texture,
}

impl MapTexture {
    /// A texture of `pixels`, which must not be empty.
    pub(super) fn new(device: &wgpu::Device, pixels: Size) -> MapTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("maplibre map"),
            size: wgpu::Extent3d { width: pixels.width, height: pixels.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        MapTexture { texture }
    }

    pub(super) fn pixels(&self) -> Size {
        Size::new(self.texture.width(), self.texture.height())
    }

    /// The ring the session renders into.
    pub(super) fn ring(&self) -> BorrowedRing {
        BorrowedRing::webgpu(BorrowedTextures {
            textures: vec![self.texture.clone()],
            size: self.pixels(),
            depth_textures: Vec::new(),
        })
    }
}
