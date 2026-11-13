# maplibre-whisker

MapLibre maps in [Whisker](https://whisker.rs) apps, drawn by mln-rs through
[maplibre-rust-ui](../README.md)'s engine into a native view of their own. It runs on iOS and
Android.

The crate is a Whisker module. Its Rust side declares the element, `Map`. Its Swift package
([`apple`](apple)) is the view on iOS and macOS, whose layer is a `CAMetalLayer`, and its Gradle
project ([`android`](android)) the view on Android, a `SurfaceView`. The map renders into either on a
thread of its own. The view hands the map its props, size, touches, mouse and trackpad through the
crate's C functions ([`ffi.rs`](src/ffi.rs)) or JNI functions ([`android.rs`](src/android.rs)), runs
its frames on a display link or the choreographer while the map wants them, and posts the events each
frame returns.

The app declares the map in its props, as maplibre-rust-ui models it ([`model.rs`](../core/src/model.rs)):

| Prop | Value |
|---|---|
| `map_style` | A style URL, or a style document |
| `camera` | `{"center": [lng, lat], "zoom": z, "bearing": b, "pitch": p}`, which moves the map when it changes |
| `content` | The app's sources and layers over the style, `{"sources": {…}, "layers": […]}` |
| `options` | Camera limits and gestures, with GL JS's `MapOptions` names (`maxZoom`, `interactive`, …) |

It hears of `on_load`, `on_moveend`, `on_map_click`, `on_contextmenu` and `on_error`, each with the
map's record, and sends commands through a `MapHandle`. A click is `on_map_click`, since Whisker keeps
`on_click` for its own gesture.

```rust
let map = MapHandle::new();
render! {
    Map(
        element_ref: map.r(),
        map_style: "https://tiles.openfreemap.org/styles/liberty",
        camera: r#"{"center": [8.54, 47.37], "zoom": 11}"#,
        content: computed(move || places(selected.get())),
        on_map_click: move |event: MapEvent| status.set(event.record()["click"].to_string()),
        style: css!(flex_grow: 1.0),
    )
}
map.fly_to(&json!({"center": [8.54, 47.37], "zoom": 14}));
```

## Running the example

[`examples/hello-map`](examples/hello-map) is a map of a few places in Zürich, which a tap names, with
buttons that fly there, fit them in view and turn the globe on. It needs mln-rs next to this repository,
as the core does, and Whisker's CLI.

```sh
cd examples/hello-map
whisker run ios
whisker run android     # with ANDROID_SERIAL naming the device when several are connected
```

With `HELLO_MAP_WALK` set, the app takes a short walk on its own, which taps the map through the same
C function as the view's touches: on a simulator,
`SIMCTL_CHILD_HELLO_MAP_WALK=1 xcrun simctl launch booted org.maplibre.rustui.hellomap`.

## Not done yet

- macOS. The view has its AppKit side, with the mouse and the trackpad, but it is unchecked:
  Whisker's macOS app takes the front of the screen when it starts.
- A surface that Android takes away, as when the app goes to the background, isn't given back to the
  map yet.
- Markers and the content's `images`.
