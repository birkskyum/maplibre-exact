# maplibre-exact

MapLibre maps in [Exact](https://github.com/expo/exact) apps.

An early prototype. It runs on macOS, iOS and Android, and [DESIGN.md](DESIGN.md) says how it works and
what is missing.

It takes the approach of [MapLibre Compose](https://github.com/maplibre/maplibre-compose). The app
declares its own sources and layers over a base style, and when its state changes, the map changes to
match. An app draws a map on a canvas:

```text
resource map = exactSurface("maplibre") as shape MapRecord
resource content = mapContent(chosen.id) as shape Content

canvas surface=maplibre(style="https://tiles.openfreemap.org/styles/liberty", longitude=8.54, latitude=47.37, zoom=11, bearing=0, pitch=0, content=content.json)
```

- `content` is the app's sources and layers in the
  [MapLibre Style Specification](https://maplibre.org/maplibre-style-spec/)'s terms, which the data
  module builds (`{"sources": {…}, "layers": […], "images": {…}}`), with images from the app's
  `assets/`. The map applies what changed, new GeoJSON data and
  changed paint and layout properties in place and the rest by adding and removing layers and sources.
- `markers` in the content holds the canvas's children at places on the map, by their `testId`, as GL
  JS's `Marker` holds an element (`{"callout": {"lngLat": [8.54, 47.38], "anchor": "bottom"}}`). A
  marker is the app's own Contract, so it keeps its taps and its accessibility, and Exact reports it
  where the map put it.
- `options` limits the camera and turns gestures off, with GL JS's `MapOptions` names (`minZoom`,
  `maxZoom`, `minPitch`, `maxPitch`, `maxBounds`, `interactive`, `dragRotate`, `pitchWithRotate`,
  `touchPitch`, `scrollZoom`).
- The map posts the events `events` lists (`load`, `moveend`, `click`, `error`) to the canvas's
  `message=` action, as GL JS's `map.on` subscribes to them.
- Its record, `exactSurface("maplibre")`, holds the camera at rest, whether everything in view has
  loaded, the last click with the content's features under it, the last failure, and the style's
  attribution as text for the app to show on the map. The app reads it as
  a shape of its own.
- Commands go in as JSON, such as
  ``postMessage(`{"command":"flyTo","center":[8.54,47.37],"zoom":14}`, "maplibre")``.

The names follow MapLibre GL JS. [`protocol.rs`](crates/maplibre-exact/src/protocol.rs) is the whole
contract, and [`examples/hello-map`](examples/hello-map) uses all of it.

## Backends

The contract doesn't depend on the engine, so a backend can be swapped without touching an app's
Contract or data module.

- `mln` is the main one and the only one so far. [mln-rs](https://github.com/louwers/mln-rs), the
  Rust port of MapLibre Native, draws on Exact's GPU canvas (wgpu), one engine for every host with a
  GPU canvas.
- Native views aren't started. MapLibre GL JS on the web and MapLibre Native's iOS and Android SDKs
  would each be an Exact package view that reads and writes the same JSON.

## Layout

- `crates/maplibre-exact` holds the protocol and the `mln` surface.
- `examples/hello-map` is an Exact app with a map, made by `exact new`.

The crate and the example find Exact and mln-rs by path, as checkouts next to this repository
(`../exact`, `../mln-rs`).

## Running the example

Clone Exact and mln-rs next to this repository and apply the patches in [`patches/`](patches). mln-rs
gets two, which give a host the offscreen frame as a texture and trust Android's system certificates
without Java. Exact gets three, which build it with Rust
1.99.1, as mln-rs needs, let its crates take ureq 3.5, as mln-rs does, and adapt `ibex2` to ureq 3.5.
They apply to Exact `85c1b9c5` and mln-rs `c558503`, the commits this was built on.

```sh
git clone https://github.com/expo/exact && git -C exact am ../maplibre-exact/patches/exact/*.patch
git clone https://github.com/louwers/mln-rs && git -C mln-rs am ../maplibre-exact/patches/mln-rs/*.patch
bun exact/scripts/exact.mjs setup
```

Then, from `maplibre-exact/examples/hello-map`:

```sh
bun exact.mjs mac --run
bun exact.mjs ios --device --run   # on a connected iPhone, with a development profile for its bundle id
bun exact.mjs android --run        # on a connected Android phone
```
