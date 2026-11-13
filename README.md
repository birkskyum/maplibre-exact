# maplibre-rust-ui

MapLibre maps for Rust UI frameworks, drawn by [mln-rs](https://github.com/louwers/mln-rs), the Rust
port of MapLibre Native.

An app declares its map the way [MapLibre Compose](https://github.com/maplibre/maplibre-compose) has it
declared: a base style, the app's own sources and layers over it, a camera and options, all in JSON with
MapLibre GL JS's names. When the app's state changes, the map changes to match. The app sends the map
commands such as `flyTo` and `fitBounds`, hears of events such as `click` and `moveend`, and reads the
map's record. That side looks the same whichever framework hosts the map.

An early prototype. [DESIGN.md](DESIGN.md) says how it works and what is missing.

## Layout

- [`core`](core) is the `maplibre-rust-ui` crate: the map an app declares
  ([`model.rs`](core/src/model.rs)), and the engine that draws it with mln-rs's runtime into what a host
  gives it, the host's own textures or a surface of a native view. It also places markers, the app's
  own views held at places on the map, as GL JS's `Marker` does, and has the map as a component with
  props in JSON ([`component.rs`](core/src/component.rs)), which hosts of native views drive.
- [`exact`](exact) is `maplibre-exact`, the map in [Exact](https://github.com/expo/exact) apps, on
  Exact's GPU canvas. It runs on macOS, iOS and Android. [Its README](exact/README.md) says how to use
  it and run its example.
- [`slint`](slint) is `maplibre-slint`, the map in [Slint](https://slint.dev) apps, on Slint's wgpu
  renderer. It runs on macOS. [Its README](slint/README.md) says how to use it.
- [`egui`](egui) is `maplibre-egui`, the map in [egui](https://www.egui.rs) apps, on egui's wgpu
  renderer. It runs on macOS. [Its README](egui/README.md) says how to use it.
- [`bevy`](bevy) is `maplibre-bevy`, the map in [Bevy](https://bevyengine.org) apps, on a UI node
  drawn with Bevy's wgpu device. It runs on macOS. [Its README](bevy/README.md) says how to use it.
- [`nsh`](nsh) is `maplibre-nsh`, the map in [Native Shell](https://nsh.dev) apps, as a native
  component whose view the map renders into. It runs on macOS and iOS. [Its README](nsh/README.md)
  says how to use it.
- [`whisker`](whisker) is `maplibre-whisker`, the map in [Whisker](https://whisker.rs) apps, as a
  Whisker module whose Swift or Kotlin view the map renders into. It runs on iOS and Android.
  [Its README](whisker/README.md) says how to use it.

Each host folder builds on the core alone, and becomes a repository of its own once the core's API
settles. Every host but Exact's drives its maps through the core's `Component`, so an engine other than
mln-rs, such as MapLibre Native or GL JS on the web, can stand in behind it without the hosts knowing.

## Building

The crates find mln-rs by path, as a checkout next to this repository (`../mln-rs`), at `55936891`
with the patches in [`patches/mln-rs`](patches/mln-rs): Apple's own SQLite, and a fix that keeps a
surface presenting on Vulkan after the map rests. The core builds and tests on its own:

```sh
git clone https://github.com/louwers/mln-rs && git -C mln-rs checkout -b maplibre-rust-ui 55936891
git -C mln-rs am ../maplibre-rust-ui/patches/mln-rs/*.patch
cd maplibre-rust-ui/core && cargo test
```
