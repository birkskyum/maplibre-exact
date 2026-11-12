# Design

## One contract, any engine

An app sees a canvas surface named `maplibre` with six named arguments, a few JSON commands in and a
few JSON events out ([`protocol.rs`](crates/maplibre-exact/src/protocol.rs)). Nothing in it names
mln-rs, so an app keeps its Contract and data module when the engine under it changes. A camera
argument moves the map only when its value changes, so the person can pan freely and the app moves
the map again by changing the value. That is how `@exact/maps` treats its camera props too.

Two kinds of backend fit behind it.

- **A GPU surface** (`mln`, built). Exact hands a surface a wgpu device, a queue and a target texture
  each frame, on every host with a GPU canvas. mln-rs renders through wgpu, so one engine and one
  style spec cover macOS, iOS, Linux, Windows and the web.
- **Native views** (not started). MapLibre GL JS on the web and MapLibre Native's iOS and Android
  SDKs, each as an implementation of an Exact package view that reads and writes the same JSON.
  Linux and Windows paint their own pixels in Exact and have no platform view to host, so this kind
  covers fewer hosts.

The open question for native views is the Contract side. A GPU surface is written
`canvas surface=maplibre(…)`, while a package view is a tag of its own (`maplibre-map style=…`).
Keeping the two interchangeable needs either Exact to let a package view be drawn by a GPU surface,
or the app to pick one spelling per platform.

## How the mln backend draws

Exact calls a surface on one thread. The surface keeps an mln-rs `Map`, a `Renderer` and an
`OffscreenTarget` there, all on Exact's device, and runs that thread's mln-rs `RunLoop` at the start
of each frame, up to 4 ms. mln-rs renders the latest update into its own `Rgba8Unorm` texture and
submits that frame itself. The surface then draws the texture into Exact's target with one
full-screen triangle, recorded into the encoder Exact submits after, so queue order makes the frame
complete before it is sampled.

The surface asks for another frame while the map loads, animates or follows a gesture, and for none
once mln-rs reports it idle. The map's animation clock is the host's (`Frame::now_ms`), so `flyTo`
and the momentum of a release follow an agent's `clock` as everything else in an Exact app does.
Tiles still load in real time.

Input comes raw from the canvas (`wants_input`) and goes to mln_gestures' `GestureRecognizer`, which
has GL JS's gesture set: drag, wheel, double click, two-finger pinch and rotate, the arrow keys and
`+`/`-`.

## What Exact would need to change

Found while building the prototype on exact2 `85c1b9c5`.

1. **A package can't ship a GPU surface.** Surfaces are registered in the app's own `gpu/` crate, so
   each app writes a three-line registry naming `maplibre_exact::mln::surface`. A package that could
   declare a surface, as it declares a view, would make the map a dependency and nothing more.
2. **Rust 1.97.0 is pinned and enforced.** The build refuses an app whose toolchain differs from the
   checkout's. mln-rs and its `mlt-core` dependency need 1.98, so the prototype moves its Exact
   checkout to 1.99.1 on a local branch.
3. **Exact version pins in vendored crates.** `hermes-lean-sys` requires `ureq = "=3.4.0"`, which
   conflicts with mln-rs's `^3.5.1` even on Apple platforms, where mln-rs never compiles ureq. The
   web build's wasm-bindgen CLI is 0.2.127 and refuses the 0.2.130 mln-rs pins.
4. **No modifier keys on pointer and wheel events, and no platform pinch.** Shift-drag to box-zoom,
   Ctrl-drag to rotate and a trackpad's magnify and rotate can't be recognized. Two-finger touch
   pinch works.
5. **`exact lock` resolves offline first and fails rather than fetching** when a new dependency needs
   a version the offline index lacks (`objc2` 0.6.5 here). A plain `cargo metadata` fetched it.

## What mln-rs would need

1. **The offscreen texture.** `OffscreenTarget` only gave its frame back as pixels on the CPU. The
   prototype adds `OffscreenTarget::color_texture()` (local branch `exact-host-texture` in
   `../mln-rs`). Rendering straight into a texture view the host owns would also drop the blit.
2. **A map on the page's thread in the browser.** Exact's GPU module runs on the page thread with the
   page's WebGPU device, while mln-rs's browser build runs its map in a Web Worker on an
   `OffscreenCanvas`.

## Where it runs

- macOS, and iOS on an iPhone 17 Pro (iOS 27.2), from the same build of the surface. On the phone the
  map took 1.5 ms of GPU time a frame while panning.
- A phone build needs a development profile for the app's bundle id that includes the phone. Exact
  uses one that is installed and doesn't create it, so the first time it is made with Xcode's
  automatic signing (`xcodebuild -allowProvisioningUpdates` on any app target with the bundle id).

## Not done yet

- The web, Linux and Windows builds. The web is blocked on the wasm-bindgen pin and on mln-rs's
  worker model.
- Markers and other app content on the map. Exact lays a canvas's children out over it and lets the
  surface place each one (`Surface::placement`), which would give markers the platform's
  accessibility for free.
- Accessibility of the map itself beyond the canvas's `aria-label`.
- Several maps on one screen share the thread's `RunLoop`, which mln-rs allows, but this is untested.
