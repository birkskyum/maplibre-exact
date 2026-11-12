//! MapLibre maps in Exact apps.
//!
//! An app draws a map with `canvas surface=maplibre(style=…, …)` and talks to
//! it in JSON: commands go in through Contract's `postMessage`, and events
//! come back through the canvas's `message=` action. [`protocol`] holds that
//! contract, which every backend keeps, so an app's Contract and data module
//! stay the same whichever engine draws the map.
//!
//! - [`mln`] draws the map with mln-rs on Exact's GPU canvas, on every host
//!   with one.
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

pub mod protocol;

#[cfg(feature = "mln")]
pub mod mln;
