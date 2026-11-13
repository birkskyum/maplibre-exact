# maplibre-gpui

MapLibre maps in [GPUI](https://www.gpui.rs) apps, drawn by mln-rs through
[maplibre-rust-ui](../README.md)'s engine. It runs on macOS so far.

The map renders on a thread of its own into a ring of IOSurfaces, with mln-rs's Metal backend, and
GPUI's Metal renderer samples the newest one where the map's view is. Nothing is copied on the way.
The view hands GPUI each frame as a `CVPixelBuffer` over its IOSurface, through GPUI's `surface`
primitive, and gives the frame back to the map once GPUI let go of it. The map composes with the rest
of the UI, and GPUI's elements draw over it. The app declares its map as maplibre-rust-ui models it
([`model.rs`](../core/src/model.rs)): a base style, its own sources and layers, a camera and options.
It sends commands such as `flyTo` and `fitBounds`, hears of events such as `click` as the entity's
events, and reads the map's record. The map takes the mouse with its modifier keys, and a trackpad's
scroll and pinch.

GPUI's Metal renderer draws only YCbCr surfaces, as video calls send them, so this needs a change to
GPUI that draws BGRA surfaces too, in [`patches/zed`](patches/zed), until Zed has it.

## Using it

A `GpuiMap` is an entity whose view is the map, filling the element it is in:

```rust
let map = cx.new(|_| maplibre_gpui::GpuiMap::new(inputs));
cx.subscribe(&map, |app, map, event: &Event, cx| {
    // a click's features are in map.read(cx).record()
})
.detach();

div().size_full().child(map.clone())
```

Commands go through the entity:

```rust
map.update(cx, |map, cx| map.command(fly_to_zurich, cx));
```

`marker_places` has where the content's markers at places go, and `at_marker` holds an element there,
in an element the map fills, with its anchor on the point. GPUI turns no elements, so a marker stays
upright. The map notifies its observers when a marker moved:

```rust
// in the content: "markers": {"station": {"lngLat": [8.5402, 47.3779], "anchor": "bottom"}}
cx.observe(&map, |_, _, cx| cx.notify()).detach();

let callouts = map.read(cx).marker_places().iter().filter(|place| place.name == "station")
    .map(|place| at_marker(place, div().child("Trains")));
div().relative().overflow_hidden().child(map.clone()).children(callouts)
```

## Running the example

[`examples/hello-map`](examples/hello-map) is a map of a few places in Zürich, which a tap names, with
buttons that fly there, fit them in view and turn the globe on, and a callout over the station. It
needs mln-rs and Zed next to this repository, Zed at the commit CI names, with the change applied:

```sh
git clone https://github.com/zed-industries/zed && git -C zed checkout dc93abce
git -C zed apply "$PWD"/maplibre-rust-ui/gpui/patches/zed/bgra-surfaces.diff
cd maplibre-rust-ui/gpui
cargo run -p hello-map-gpui
cargo test -p hello-map-gpui -- --ignored   # a short walk offscreen, saved in target/tmp/hello-map-walk
```

The walk drives the app in GPUI's headless context, whose Metal renderer draws its frames into images,
and shows no window. The map loads, flies to Zürich, a tap names Zürich HB, and the places fit in view.

The example runs on GPUI's Mac platform itself rather than through `gpui_platform`, which takes the
web platform too, whose exact `unicode-properties` mln-rs can't share.

## Not done yet

- Windows and Linux, where the same works with another kind of shared image and another change to
  GPUI, a DXGI shared texture for GPUI's DirectX renderer and a dmabuf for its wgpu one.
- iOS, which GPUI's Metal renderer serves too.
- Markers that turn with the map or lie on it as it pitches.
