# maplibre-exact

MapLibre maps in [Exact](https://github.com/expo/exact) apps.

An app draws a map on a canvas and talks to it in JSON:

```text
canvas surface=maplibre(style="https://tiles.openfreemap.org/styles/liberty", longitude=8.54, latitude=47.37, zoom=11, bearing=0, pitch=0) message=heard
```

```text
postMessage(`{"command":"flyTo","center":[8.54,47.37],"zoom":14}`, "maplibre")
```

The map posts `load`, `moveend`, `click` and `error` events to the canvas's `message=` action, and its
camera is the record `exactSurface("maplibre")`. The names follow MapLibre GL JS.
[`crates/maplibre-exact/src/protocol.rs`](crates/maplibre-exact/src/protocol.rs) is the whole contract.

## Backends

The contract doesn't depend on the engine, so a backend can be swapped without touching an app's
Contract or data module.

- `mln`, the one there is: [mln-rs](https://github.com/louwers/mln-rs), the Rust port of MapLibre
  Native, drawing on Exact's GPU canvas (wgpu). One engine for every host with a GPU canvas.
- Native views, not started: MapLibre GL JS on the web, MapLibre Native's iOS and Android SDKs, as an
  Exact package view that reads and writes the same JSON.

## Layout

- `crates/maplibre-exact`: the protocol and the `mln` surface.
- `examples/hello-map`: an Exact app with a map, made by `exact new`.

The crate and the example find Exact and mln-rs by path, as checkouts next to this repository
(`../exact`, `../mln-rs`).

## Running the example

From `examples/hello-map`, with Exact's tools set up (`bun ../../../exact/scripts/exact.mjs setup`):

```sh
bun exact.mjs mac --run
```
