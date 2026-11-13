//! Tells the build scripts of dependent crates where `maplibre.slint` is, so
//! that an app's Slint compiler imports it as `@maplibre/maplibre.slint`.

fn main() {
    let ui = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("set by Cargo")).join("ui");
    println!("cargo::metadata=ui={}", ui.display());
    println!("cargo::rerun-if-changed=build.rs");
}
