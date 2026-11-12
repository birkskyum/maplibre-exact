//! Hello Map's GPU module, which draws the MapLibre map.

use exact_gpu::Registry;

/// The module's surfaces.
pub static REGISTRY: Registry =
    Registry { surfaces: &[("maplibre", maplibre_exact::protocol::ARITY, maplibre_exact::mln::surface)], shaders: &[] };

exact_gpu::module!(REGISTRY);
