//! A texture for a host that draws with wgpu to lend the render session as a
//! ring of one, which the host then draws. A session the host drives renders
//! only while the host services it, before the host's own draw goes to the
//! device's queue, so the queue orders the host's draw after the frame and the
//! next frame after the host's draw.

use mln_runtime::BorrowedRing;
use mln_util::size::Size;
use mln_webgpu::BorrowedTextures;

/// Premultiplied, as mln-rs draws.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// An `Rgba8Unorm` texture, premultiplied as mln-rs draws, that render passes
/// draw into and shaders sample, as it is or through an `Rgba8UnormSrgb` view.
pub struct MapTexture {
    pub texture: wgpu::Texture,
}

impl MapTexture {
    /// A texture of `pixels`, which must not be empty.
    #[must_use]
    pub fn new(device: &wgpu::Device, pixels: Size) -> MapTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("maplibre map"),
            size: wgpu::Extent3d { width: pixels.width, height: pixels.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            // A host that samples colors as linear, as Bevy does, reads the
            // map's sRGB pixels through an sRGB view.
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        });
        MapTexture { texture }
    }

    #[must_use]
    pub fn pixels(&self) -> Size {
        Size::new(self.texture.width(), self.texture.height())
    }

    /// The ring the session renders into.
    #[must_use]
    pub fn ring(&self) -> BorrowedRing {
        BorrowedRing::webgpu(BorrowedTextures {
            textures: vec![self.texture.clone()],
            size: self.pixels(),
            depth_textures: Vec::new(),
        })
    }
}
