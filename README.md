# maplibre-rust-ui

MapLibre maps for Rust UI frameworks, drawn by [mln-rs](https://github.com/louwers/mln-rs), the Rust
port of MapLibre Native.

An app declares its map as a base style, the app's own sources and layers over it, a camera and
options, all in JSON with MapLibre GL JS's names. The app sends the map commands such as `flyTo` and
`fitBounds`, hears of events such as `click` and `moveend`, and reads the map's record. That side
looks the same whichever framework hosts the map.

**[The docs](https://birkskyum.github.io/maplibre-rust-ui/)** have how to use it, [the map model](https://birkskyum.github.io/maplibre-rust-ui/model/), each host, and
[how it works](https://birkskyum.github.io/maplibre-rust-ui/design/).

## Layout

- [`core`](core) is the `maplibre-rust-ui` crate, the map an app declares and the engine that draws
  it with mln-rs's runtime into what a host gives it ([API](https://birkskyum.github.io/maplibre-rust-ui/api/maplibre_rust_ui/)).
- [`slint`](slint), [`egui`](egui), [`bevy`](bevy) and [`gpui`](gpui) are the map in
  [Slint](https://birkskyum.github.io/maplibre-rust-ui/hosts/slint/), [egui](https://birkskyum.github.io/maplibre-rust-ui/hosts/egui/), [Bevy](https://birkskyum.github.io/maplibre-rust-ui/hosts/bevy/) and
  [GPUI](https://birkskyum.github.io/maplibre-rust-ui/hosts/gpui/) apps, on macOS.
- [`nsh`](nsh) is the map in [Native Shell](https://birkskyum.github.io/maplibre-rust-ui/hosts/nsh/) apps, on macOS and iOS.
- [`whisker`](whisker) is the map in [Whisker](https://birkskyum.github.io/maplibre-rust-ui/hosts/whisker/) apps, on iOS and Android.
- [`exact`](exact) is the map in [Exact](https://birkskyum.github.io/maplibre-rust-ui/hosts/exact/) apps, on macOS, iOS and Android.
- [`docs`](docs) is the docs site, built with [Zensical](https://zensical.org).

An early prototype.

## Building

The crates find mln-rs by path, as a checkout next to this repository (`../mln-rs`), at `1102a74c`. The
core builds and tests on its own:

```sh
git clone https://github.com/louwers/mln-rs && git -C mln-rs checkout -b maplibre-rust-ui 1102a74c
cd maplibre-rust-ui/core && cargo test
```

The docs build with `uvx zensical serve`, from this folder.

## Releasing

The core and the Slint, egui, Bevy, Native Shell and Whisker hosts go on crates.io. The Release
workflow, run by hand, publishes each whose version in its `Cargo.toml` isn't there yet, the core
first, and tags it. A new version of the core needs the hosts' `maplibre-rust-ui` versions raised with
it, since each host is a workspace of its own.
