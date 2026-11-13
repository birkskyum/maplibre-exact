//! MapLibre maps in Exact apps, drawn by mln-rs on Exact's GPU canvas.
//!
//! An app draws a map with `canvas surface=maplibre(style=…, …)` and talks to
//! it in JSON: commands go in through Contract's `postMessage`, and events
//! come back through the canvas's `message=` action. [`protocol`] binds the
//! canvas's arguments to the map maplibre-rust-ui models, and [`mln`] is the
//! surface, which draws the map on every host with a GPU canvas.
//!
//! Exact registers surfaces in the app's own GPU module, so the app names
//! this one there:
//!
//! ```ignore
//! pub static REGISTRY: exact_gpu::Registry = exact_gpu::Registry {
//!     surfaces: &[("maplibre", maplibre_exact::protocol::ARITY, maplibre_exact::mln::surface)],
//!     shaders: &[],
//! };
//! exact_gpu::module!(REGISTRY);
//! ```

pub mod mln;
pub mod protocol;
