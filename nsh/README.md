# maplibre-nsh

MapLibre maps in [Native Shell](https://nsh.dev) apps, drawn by mln-rs through
[maplibre-rust-ui](../README.md)'s engine into a native view of their own. It runs on macOS so far.

The crate registers a native component, `maplibre.map`. On macOS its view's layer is a `CAMetalLayer`,
which the map renders into on a thread of its own, and the view takes the mouse and the trackpad
itself, with GL JS's gestures, a trackpad's pinch and rotate among them. The app declares the map in
the component's props, as maplibre-rust-ui models it ([`model.rs`](../core/src/model.rs)), sends it
commands such as `flyTo` and `fitBounds`, and hears of its events, each with the map's record.

| Prop | Value |
|---|---|
| `style` | A style URL, or a style document |
| `camera` | `{"center": [lng, lat], "zoom": z, "bearing": b, "pitch": p}`, which moves the map when it changes |
| `content` | The app's sources and layers over the style, `{"sources": {…}, "layers": […]}` |
| `options` | Camera limits and gestures, with GL JS's `MapOptions` names (`maxZoom`, `interactive`, …) |
| `events` | The events it posts, `["load", "moveend", "click", "contextmenu", "error"]`, all of them by default |

## Using it

The app registers the map before it runs, or in its `#[native_components(macos)]` function:

```rust
nsh_platform_macos::set_components(maplibre_nsh::register);
```

and places it like any element:

```rust
let map = element_ref();
maplibre_nsh::map()
    .element_ref(&map)
    .prop("style", json!("https://tiles.openfreemap.org/styles/liberty"))
    .prop("camera", json!({"center": [8.54, 47.37], "zoom": 11}))
    .prop("content", move || places(selected.get()))
    .on_native_event(move |event: &NativeEvent| {
        if event.name.as_ref() == "click" {
            status.set(event.data["click"]["features"].to_string());
        }
    })
    .flex_1()
```

Commands go through the element's ref, with GL JS's camera options:

```rust
map.command("flyTo", json!({"center": [8.54, 47.37], "zoom": 14}));
```

A TSX app registers it from its Rust crate (`nsh.json` `"rust"`) and declares it with
`nativeComponent("maplibre.map", { props: ["style", "camera", "content", "options", "events"] })`.

## Running the example

[`examples/hello-map`](examples/hello-map) is a map of a few places in Zürich, which a tap names, with
buttons that fly there, fit them in view and turn the globe on. It needs mln-rs next to this repository,
as the core does.

```sh
cargo run -p hello-map-nsh
HELLO_MAP_SNAPSHOTS=/tmp/hello-map cargo run -p hello-map-nsh   # a short walk, saved as PNG files
```

The walk's window opens behind the others, on a second display where there is one, since a covered
window gets no frames.

## Not done yet

- iOS, where a Rust component gives a `UIView`, the same way.
- Android, where Native Shell takes components from Kotlin or Java, so the map needs a small Kotlin
  view that hands its `Surface` to Rust.
- Windows and Linux.
- Markers and the content's `images`.
