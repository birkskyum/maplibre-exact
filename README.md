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
  own views held at places on the map, as GL JS's `Marker` does.
- [`exact`](exact) is `maplibre-exact`, the map in [Exact](https://github.com/expo/exact) apps, on
  Exact's GPU canvas. It runs on macOS, iOS and Android. [Its README](exact/README.md) says how to use
  it and run its example.

- [`slint`](slint) is `maplibre-slint`, the map in [Slint](https://slint.dev) apps, on Slint's wgpu
  renderer. It runs on macOS. [Its README](slint/README.md) says how to use it.

Hosts for [Whisker](https://whisker.rs) and [Native Shell](https://nsh.dev) come next. Each host folder
builds on the core alone, and becomes a repository of its own once the core's API settles.

## Building

The crates find mln-rs by path, as a checkout next to this repository (`../mln-rs`), at `55936891` or
later. The core builds and tests on its own:

```sh
git clone https://github.com/louwers/mln-rs
cd maplibre-rust-ui/core && cargo test
```
