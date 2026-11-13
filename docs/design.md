# Design

## One model, any host

An app declares a map ([the map model](model.md)): a base style, its own content over it, a
camera and options. It sends commands as JSON, hears of the events it listens for, and reads the map's
record, all with MapLibre GL JS's names. Nothing in that model names a UI framework or an engine. A
camera moves the map only when its value changes, so the person can pan freely and the app moves the
map again by changing the value. That is how `@exact/maps` treats its camera props too.

The core's [`Engine`](https://github.com/birkskyum/maplibre-rust-ui/blob/main/core/src/engine.rs) draws that map, and each UI framework has a host that gives
the engine somewhere to draw and its input, and gives the app the model in the framework's own terms.
In Exact the map is a canvas surface named `maplibre` with nine named arguments, commands come in
through `postMessage`, events go out to the canvas's `message=` action, and the app reads the record
as `exactSurface("maplibre")` ([`protocol.rs`](https://github.com/birkskyum/maplibre-rust-ui/blob/main/exact/src/protocol.rs)).

The engine draws with mln-rs, through its runtime, whose API mirrors maplibre-native-ffi's C API one
to one. MapLibre Native can stand in for it through maplibre-native-ffi, which takes the same calls,
for an app that wants the more mature engine. That isn't built yet.

## Content, the way MapLibre Compose does it

The app declares its own sources and layers over the base style, and the map follows that
declaration as the app's state changes. In MapLibre Compose the declaration is a composable block. In
Exact it is the `content` argument, the style specification's JSON, which the data module builds from
the app's state, typed in TypeScript.

- A layer draws above the base style and above the content's layers before it, under the layer its
  `beforeId` names, or under the base style's first layer of the type its `beforeType` names, so
  `"beforeType": "symbol"` draws it under the labels whatever the style calls them.
- When the content changes, the map does the least it can. A GeoJSON source takes new data in place,
  and a layer takes changed paint and layout properties, filter and zoom range in place. A changed
  source type or tiles, a changed layer type or source, or a new order removes and adds those layers
  and sources again.
- Content is applied once the style has loaded, and again after a new style loads, so switching base
  styles keeps it.
- `projection`, `sky`, `light` and `terrain` set the style's root properties over the base style's,
  which come back when the content drops them, and `state` sets the values its `global-state`
  expressions read. What an engine doesn't draw, as mln-rs doesn't draw terrain, is reported in the
  record's `failure` and an `error` event, and doesn't refuse the canvas, since another engine may
  draw it.
- `images` names the app's images that layers use, such as an `icon-image`, by their paths under
  `assets/`. The surface asks Exact for each, decodes it with mln-rs and adds it to the style.
- A click queries the content's layers at the point, widened by `hitPadding`, and the record's `click`
  lists their features topmost first, with `id`, `layer`, `source`, `sourceLayer` and `properties`,
  and its `lngLat` and `point` as GL JS's `MapMouseEvent` has them (`{lng, lat}`, `{x, y}`). A
  secondary click or a touch held still for half a second is a `contextmenu`, as a browser makes
  one, and its release is no click. A cluster's feature carries `clusterExpansionZoom`, the zoom
  GL JS's `getClusterExpansionZoom` answers, so the app can zoom in to split it with one command.
- `markers` holds children of the canvas at places on the map, by their `testId`, with GL JS's
  `Marker` options `lngLat`, `anchor`, `offset`, `rotation`, `rotationAlignment` and
  `pitchAlignment`. The rest of the canvas's children stay where the layout puts them, as controls
  over the map do.

Events reach Contract as plain names because Contract has no JSON reader, and the details go in the
record, which the app reads as a shape of its own. The record's camera is the one at rest, so the
record changes when something happens and not on every frame of a gesture.

## Markers

A marker is a child of the map's canvas in the app's own Contract, such as a callout with a button,
laid out at its own size (`position="absolute"`). Exact's GPU canvas can hand a surface each of its
children and ask, after every frame, where the surface put each one (`ChildrenMode::Each` and
`Surface::placement`). The map answers with the marker's anchor at its place on the map as drawn
that frame, and Exact hit-tests the child and reports it to accessibility and to an agent there, so
a marker is tapped, read out and driven like any other node. A marker whose place is off the canvas
or behind the globe is hidden, which takes it out of hit-testing and accessibility too.

A placement is a homography, so a marker turns and tilts as GL JS's does: `rotationAlignment: "map"`
turns it with the bearing and `pitchAlignment: "map"` lays it on the map as it pitches, both about
its anchor. A marker without `lngLat` stays where the layout put it and only turns, which is a
compass: the example's turns its needle with the map, flattens it with the pitch as GL JS's
`visualizePitch` does, and puts north back up when tapped.

Who draws the marker's pixels depends on the host. On Apple the host paints each child into a
texture and the map draws the placed ones over its frame, in the same pass, so a marker never lags
the map. On Android (and Linux and the web) the host draws the child itself at the placement. The
canvas asks for children one by one only while the content has markers, since capturing each costs
the host more than its ordinary overlay.

## How the engine draws

The map lives on mln-rs's runtime, on a thread of the runtime's own. The host's thread sends it
commands, which run there in order and report later through their completions. The map publishes a
snapshot of its state after each, which the host's thread reads at any time without waiting: markers,
clicks and the gestures take the camera from it. Each frame the engine demands a frame of what changed.

A render session attached to the map draws it into what the host gives it.

- **The host's textures.** Exact hands a surface a wgpu device and a target texture each frame, so its
  host makes a texture of its own on that device and lends it to the session as a ring of one. The
  session renders on the surface's thread, only while the surface services it, so a frame goes to the
  device's queue before the surface's draw, which samples the texture into Exact's target with one
  full-screen triangle. The next frame renders on a later service, after Exact submitted that draw.
- **A surface.** A native view's Metal layer or Android surface takes the session's frames, which it
  renders on a thread of its own. Native Shell gives the map a native view of its own, and on macOS its
  host makes that view's layer a `CAMetalLayer` and hands the session a wgpu surface of it, made on the
  main thread. Whisker's views are Swift and Kotlin, so its host's Swift view makes its layer a
  `CAMetalLayer` and hands it to the crate's C functions, with its props, size and input. Both drive
  the map through the core's `Component`, which takes props, commands and
  input and returns the events to post. A covered macOS window gets no drawables, so the map draws
  nothing there, and a click finds its features in the last frame drawn.

The map's animation clock is the host's, so `flyTo` and the momentum of a release follow an agent's
`clock` as everything else in an Exact app does. The runtime has no call for it yet, so the engine sets
the core map's frame time through `Map::with_core`. Tiles still load in real time. The host asks for
another frame while the map has updates it hasn't drawn, loads, animates or follows a gesture, and for
none once everything has drawn.

A click asks the session for the features the content's layers drew there, and for a cluster's
expansion zoom, and reaches the app once both answered, a frame or two later.

Input goes to mln_gestures' `GestureRecognizer`, which has GL JS's gesture set of drag, wheel, double
click, two-finger pinch and rotate, the arrow keys and `+`/`-`, and moves the map with the camera
deltas maplibre-native-ffi's C API takes.

## Against MapLibre Compose

| Compose guide | Here |
|---|---|
| Composition, a base style plus declared sources and layers, diffed | Done, as `content` |
| Layers, with GeoJSON and tiled sources, expressions and `beforeId` | Done, and `beforeType` draws under the base style's first layer of a type, as `Anchor.Below` does |
| Interaction, with clicks on the map and its layers and hit padding | Done, with long presses and secondary clicks as `contextmenu`, and `options` turns off rotating, pitching, scroll zoom or every gesture, keeping clicks |
| Camera state and `animateTo` | `jumpTo`, `easeTo`, `flyTo`, `fitBounds` and the record, with camera `padding` and zoom, pitch and bounds limits in `options` |
| Styling, switching styles at run time and keeping content | Done, with the `setStyle` command or a new `style` |
| Clustering | Done, with the GeoJSON source's cluster options and each clicked cluster's expansion zoom |
| Feature queries and feature state | Clicks query the content's layers; global state in the content's `state` drives expressions, as GL JS's `setGlobalStateProperty` does; no feature state |
| Images and icons | Done, as content `images` from the app's `assets/`, which Exact delivers to the surface |
| Markers and popups, as GL JS's `Marker` and `Popup` | Done, as content `markers` holding the canvas's Contract children, tappable and accessible |
| Controls, the compass, scale bar and attribution | A compass is a Contract child turned as a marker, and the record's `attribution` is text the app shows over the map; no scale bar |
| Terrain, hillshade and the globe | Hillshade and color relief layers, and the globe as the content's `projection`; mln-rs draws no 3D terrain, so the content's `terrain` is reported as a failure, without refusing the canvas |
| Location and the location puck | The app's own, from Exact's `@exact/geolocation`, with the dot a marker; the example's locate button flies there |
| Offline, requests, snapshotter | Not yet |

## What Exact would need to change

Found while building the prototype on exact2 `85c1b9c5`, which it has since moved to `09824dea`.
Filed: [#425](https://github.com/expo/exact/issues/425) (10), [#426](https://github.com/expo/exact/issues/426) (2, 3), [#427](https://github.com/expo/exact/issues/427) (1) and [#428](https://github.com/expo/exact/issues/428) (4).

1. **A package can't ship a GPU surface.** Surfaces are registered in the app's own `gpu/` crate, so
   each app writes a three-line registry naming `maplibre_exact::mln::surface`. A package that could
   declare a surface, as it declares a view, would make the map a dependency and nothing more.
2. **Rust 1.97.0 is pinned and enforced.** The build refuses an app whose toolchain differs from the
   checkout's. mln-rs and its `mlt-core` dependency need 1.98, so the prototype moves its Exact
   checkout to 1.99.1 ([`exact/patches`](https://github.com/birkskyum/maplibre-rust-ui/tree/main/exact/patches)).
3. **Exact version pins in vendored crates.** `hermes-lean-sys`, `ibex2` and the Hermes installer
   require `ureq = "=3.4.0"`, because `ibex2` uses ureq's `unversioned` transport API, which changes
   between minor versions. mln-rs needs `^3.5.1`, which Cargo resolves even on Apple platforms, where
   mln-rs never compiles ureq. With the pins relaxed, `ibex2` needs one line for ureq 3.5
   (`NextTimeout` gained two fields), in [`exact/patches`](https://github.com/birkskyum/maplibre-rust-ui/tree/main/exact/patches). The web build's
   wasm-bindgen CLI is 0.2.127 and refuses the 0.2.130 mln-rs pins.
4. **No modifier keys on pointer and wheel events, and no platform pinch.** Shift-drag to box-zoom,
   Ctrl-drag to rotate and a trackpad's magnify and rotate can't be recognized. Two-finger touch
   pinch works.
5. **`exact lock` resolves offline first and fails rather than fetching** when a new dependency needs
   a version the offline index lacks (`objc2` 0.6.5 here). A plain `cargo metadata` fetched it. It
   also keeps a locked version that a path dependency has since ruled out (`thiserror` 2.0.20, once
   mln-rs asked for `^2.0.21`) and fails, where `cargo update -p thiserror` raises just that one.
6. **An Android build of an app with no icons fails** writing `icons/res.key` into a folder it never
   made. The example declares an icon.
7. **A message to a canvas with no `message=` action** is ignored on Apple and refused on Android
   (`NoHandler`). The map posts only the events the app lists, so an app without the action works on
   both.
8. **A surface's record shares the agent's view of a canvas with the host's own fields**, so a record
   field named `error` reads as a refused canvas and stops a drive. The record says `failure`.
9. **An app's own workspace isn't given a new platform's crate** by the native verb, only by
   `exact update`, which a GPU module's app has to know to run.
10. **iOS hands a surface the wrong pixels for its children one by one** (`ChildrenMode::Each`).
    The GPU capture (`Shadow`) reads its target back before `CARenderer` has drawn the child, so
    each child gets what the last capture of its size left there, or nothing in a new target: the
    compass showed the locate button's arrow, and the callout was blank. It shows on an iPhone 17
    Pro (iOS 27.2) and the iOS 26.5 simulator, and `EXACT_CAPTURE=cpu` cures it. The prototype
    captures each child on the CPU ([`exact/patches`](https://github.com/birkskyum/maplibre-rust-ui/tree/main/exact/patches)), which costs little for
    children the size of markers.

## What mln-rs would need

1. **A frame time in the runtime.** The engine sets the host's clock through `Map::with_core`, which
   reaches past the runtime's API.
2. **A map on the page's thread in the browser.** Exact's GPU module runs on the page thread with the
   page's WebGPU device, while mln-rs's browser build runs its map in a Web Worker on an
   `OffscreenCanvas`.
3. **A view's surface on a session of its own thread on macOS.** A session builds a window's surface
   on the thread that renders, and wgpu takes an AppKit view's layer only on the main thread, so a
   session on `Driver::CoreWorker` given `Surface::window` of an `NSView` panics ("can only access
   NSView on the main thread"). Native Shell's host makes the wgpu surface from the view's layer
   itself, on the main thread, and hands it over with `Surface::wgpu`.

Five needs are met.

- A host renders into its own textures with `BorrowedTarget`, on every mln-rs backend.
- mln-rs trusts Android's system certificates when the app gives it no Java VM
  ([louwers/mln-rs#307](https://github.com/louwers/mln-rs/pull/307)).
- mln-rs links Apple's own SQLite on Apple's platforms, as Exact's own crates do
  ([louwers/mln-rs#344](https://github.com/louwers/mln-rs/pull/344)). The SQLite it built in probed
  filesystem types with `statfs`, which Apple counts as a disk-space API whose use an app's privacy
  manifest must give a reason for, and none of Apple's reasons fits, so Exact's iOS build refused the
  app.
- A surface keeps presenting on Vulkan after the map rests
  ([louwers/mln-rs#343](https://github.com/louwers/mln-rs/pull/343)). After ten seconds without a
  frame the WebGPU surface asks to be reconfigured, and with its size unchanged it dropped the frame's
  acquired texture unpresented. Metal takes such a drawable back and Vulkan doesn't, so on Android
  each rest of the map cost the swapchain an image, until none was left and the map stopped drawing
  ("dequeueBuffer timed out").
- The gesture recognizer's camera trait and events are public again, and its options have switches
  for rotating, pitching and scroll zoom
  ([louwers/mln-rs#350](https://github.com/louwers/mln-rs/pull/350)). A host on the runtime has the
  map on the runtime's thread, so its camera reads the map's snapshot and sends the recognizer's
  deltas as commands, which takes implementing `GestureCamera` outside mln-rs.

## What GPUI would need

1. **BGRA surfaces in its Metal renderer.** GPUI draws an IOSurface where an element is, without a
   copy, through its `surface` primitive, but only in the YCbCr format that video calls send, which
   would smear the colours of a map's labels. The prototype draws BGRA surfaces too, with a second
   pipeline whose fragment shader samples one texture ([`gpui/patches/zed`](https://github.com/birkskyum/maplibre-rust-ui/tree/main/gpui/patches/zed)).
2. **Surfaces in its DirectX and wgpu renderers.** Its DirectX renderer takes surfaces and draws
   none, and its wgpu renderer, on Linux and the web, takes none. mln-rs shares DXGI textures and
   dmabufs, which those renderers would draw as the Metal one draws IOSurfaces.

## Where it runs

- Slint, egui, Bevy, GPUI and Native Shell on macOS, and Whisker on the iOS simulator (iOS 26.5): the style, the
  content's places, `flyTo`, a tap naming a place and `fitBounds`, each in a scripted walk. Native
  Shell on the iOS simulator too, without the tap. Whisker on an Android 15 emulator, with the
  emulator's own touches: the buttons, a tap naming a place and a drag, with pauses past the
  surface's ten seconds between them. The station's callout in Slint, egui, Bevy, GPUI, Whisker and
  Native Shell (on iOS), and on Whisker's iOS and Android also turned and tilted with the map. GPUI's
  walk runs in its headless context, whose Metal renderer draws the map's IOSurfaces into its images.

- Exact on macOS, iOS on an iPhone 17 Pro (iOS 27.2) and Android on a Pixel 9a, from the same surface: the
  style, the content's route and pins, `flyTo`, `fitBounds`, a tap selecting a pin, and the
  attribution. The callout marker and its button, clusters zooming in, camera padding and a
  right-click on macOS, and the callout, the compass and a long press on Android and on the
  iPhone (the compass turning there on the iOS 26.5 simulator, since the phone's Metal HUD sits
  over it). The globe, the relief and the located dot on macOS and Android. On the iPhone the map took 1.5 ms of GPU time a frame while panning. Android draws with
  wgpu's Vulkan backend, and mln-rs fetches over its ureq transport there, where Apple uses
  NSURLSession.
- A phone build needs a development profile for the app's bundle id that includes the phone. Exact
  uses one that is installed and doesn't create it, so the first time it is made with Xcode's
  automatic signing (`xcodebuild -allowProvisioningUpdates` on any app target with the bundle id).

## Not done yet

- The web, Linux and Windows builds. The web is blocked on the wasm-bindgen pin and on mln-rs's
  worker model.
- The rest of the table above.
- Accessibility of the map itself beyond the canvas's `aria-label`.
- Several maps on one screen each have a runtime of their own, and in Exact the record's name,
  `maplibre`, belongs to the first map.
- Slint, egui, Bevy and GPUI beyond macOS, Native Shell on Android, Whisker on macOS, and Whisker on an
  Android phone.
- iced, which is on an older wgpu, so its host needs mln-rs's exported textures, as GPUI's has.
- MapLibre Native through maplibre-native-ffi, and GL JS on the web, as engines behind `Component`.
