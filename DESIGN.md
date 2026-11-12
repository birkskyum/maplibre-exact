# Design

## One contract, any engine

An app sees a canvas surface named `maplibre` with nine named arguments, JSON commands in, the names
of the events it listens for out to `message=`, and a record it reads as `exactSurface("maplibre")`
([`protocol.rs`](crates/maplibre-exact/src/protocol.rs)). Nothing in it names mln-rs, so an app keeps
its Contract and data module when the engine under it changes. A camera argument moves the map only
when its value changes, so the person can pan freely and the app moves the map again by changing the
value. That is how `@exact/maps` treats its camera props too.

Two kinds of backend fit behind it.

- **A GPU surface** (`mln`, built). Exact hands a surface a wgpu device, a queue and a target texture
  each frame, on every host with a GPU canvas. mln-rs renders through wgpu, so one engine and one
  style spec cover macOS, iOS, Linux, Windows and the web. This is the main backend.
- **Native views** (not started). MapLibre GL JS on the web and MapLibre Native's iOS and Android
  SDKs, each as an implementation of an Exact package view that reads and writes the same JSON. The
  content is the style specification's own JSON, which GL JS's `addSource`, `addLayer` and
  `setPaintProperty` take as it is. Linux and Windows paint their own pixels in Exact and have no
  platform view to host, so this kind covers fewer hosts.

The open question for native views is the Contract side. A GPU surface is written
`canvas surface=maplibre(…)`, while a package view is a tag of its own (`maplibre-map style=…`).
Keeping the two interchangeable needs either Exact to let a package view be drawn by a GPU surface,
or the app to pick one spelling per platform.

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
- `images` names the app's images that layers use, such as an `icon-image`, by their paths under
  `assets/`. The surface asks Exact for each, decodes it with mln-rs and adds it to the style.
- A click queries the content's layers at the point, widened by `hitPadding`, and the record's `click`
  lists their features topmost first, with `id`, `layer`, `source`, `sourceLayer` and `properties`.

Events reach Contract as plain names because Contract has no JSON reader, and the details go in the
record, which the app reads as a shape of its own. The record's camera is the one at rest, so the
record changes when something happens and not on every frame of a gesture.

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
has GL JS's gesture set of drag, wheel, double click, two-finger pinch and rotate, the arrow keys and
`+`/`-`.

## Against MapLibre Compose

| Compose guide | Here |
|---|---|
| Composition, a base style plus declared sources and layers, diffed | Done, as `content` |
| Layers, with GeoJSON and tiled sources, expressions and `beforeId` | Done, and `beforeType` draws under the base style's first layer of a type, as `Anchor.Below` does |
| Interaction, with clicks on the map and its layers and hit padding | Done, and `options` turns off rotating, pitching, scroll zoom or every gesture, keeping clicks; no long press |
| Camera state and `animateTo` | `jumpTo`, `easeTo`, `flyTo`, `fitBounds` and the record, with zoom, pitch and bounds limits in `options`; no camera padding |
| Styling, switching styles at run time and keeping content | Done, with the `setStyle` command or a new `style` |
| Clustering | The GeoJSON source's cluster options pass through; expanding a cluster isn't there |
| Feature queries and feature state | Not yet |
| Images and icons | Done, as content `images` from the app's `assets/`, which Exact delivers to the surface |
| Controls, the compass, scale bar and attribution | The record's `attribution`, which the app shows over the map; no compass or scale bar |
| Terrain and hillshade | Up to the style; untested on mln-rs |
| Location, offline, requests, snapshotter | Not yet |

## What Exact would need to change

Found while building the prototype on exact2 `85c1b9c5`.

1. **A package can't ship a GPU surface.** Surfaces are registered in the app's own `gpu/` crate, so
   each app writes a three-line registry naming `maplibre_exact::mln::surface`. A package that could
   declare a surface, as it declares a view, would make the map a dependency and nothing more.
2. **Rust 1.97.0 is pinned and enforced.** The build refuses an app whose toolchain differs from the
   checkout's. mln-rs and its `mlt-core` dependency need 1.98, so the prototype moves its Exact
   checkout to 1.99.1 ([`patches/exact`](patches/exact)).
3. **Exact version pins in vendored crates.** `hermes-lean-sys`, `ibex2` and the Hermes installer
   require `ureq = "=3.4.0"`, because `ibex2` uses ureq's `unversioned` transport API, which changes
   between minor versions. mln-rs needs `^3.5.1`, which Cargo resolves even on Apple platforms, where
   mln-rs never compiles ureq. With the pins relaxed, `ibex2` needs one line for ureq 3.5
   (`NextTimeout` gained two fields), in [`patches/exact`](patches/exact). The web build's
   wasm-bindgen CLI is 0.2.127 and refuses the 0.2.130 mln-rs pins.
4. **No modifier keys on pointer and wheel events, and no platform pinch.** Shift-drag to box-zoom,
   Ctrl-drag to rotate and a trackpad's magnify and rotate can't be recognized. Two-finger touch
   pinch works.
5. **`exact lock` resolves offline first and fails rather than fetching** when a new dependency needs
   a version the offline index lacks (`objc2` 0.6.5 here). A plain `cargo metadata` fetched it.
6. **An Android build of an app with no icons fails** writing `icons/res.key` into a folder it never
   made. The example declares an icon.
7. **A message to a canvas with no `message=` action** is ignored on Apple and refused on Android
   (`NoHandler`). The map posts only the events the app lists, so an app without the action works on
   both.
8. **A surface's record shares the agent's view of a canvas with the host's own fields**, so a record
   field named `error` reads as a refused canvas and stops a drive. The record says `failure`.
9. **An app's own workspace isn't given a new platform's crate** by the native verb, only by
   `exact update`, which a GPU module's app has to know to run.

## What mln-rs would need

1. **The offscreen texture.** `OffscreenTarget` only gave its frame back as pixels on the CPU. The
   prototype adds `OffscreenTarget::color_texture()` ([`patches/mln-rs`](patches/mln-rs)). Rendering
   straight into a texture view the host owns would also drop the blit.
2. **A map on the page's thread in the browser.** Exact's GPU module runs on the page thread with the
   page's WebGPU device, while mln-rs's browser build runs its map in a Web Worker on an
   `OffscreenCanvas`.

## Where it runs

- macOS, iOS on an iPhone 17 Pro (iOS 27.2) and Android on a Pixel 9a, from the same surface. On the
  iPhone the map took 1.5 ms of GPU time a frame while panning. Android draws with wgpu's Vulkan
  backend, and mln-rs fetches over its ureq transport there, where Apple uses NSURLSession.
- A phone build needs a development profile for the app's bundle id that includes the phone. Exact
  uses one that is installed and doesn't create it, so the first time it is made with Xcode's
  automatic signing (`xcodebuild -allowProvisioningUpdates` on any app target with the bundle id).

## Not done yet

- The web, Linux and Windows builds, and the map itself on Android, which so far ran offline. The web is blocked on the wasm-bindgen pin and on mln-rs's
  worker model.
- The rest of the table above.
- Markers drawn by the app's Contract. Exact lays a canvas's children out over it and lets the surface
  place each one (`Surface::placement`), which would give markers the platform's accessibility.
- Accessibility of the map itself beyond the canvas's `aria-label`.
- Several maps on one screen share the thread's `RunLoop`, which mln-rs allows, but the record's
  name, `maplibre`, belongs to the first map.
