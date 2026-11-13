# Hosts

A host gives the map somewhere to draw and its input, and gives the app [the map model](../model.md)
in its framework's own terms. Each builds on the core alone, and becomes a repository of its own once
the core's API settles. Every host but Exact's drives its maps through the core's
[`Component`](https://birkskyum.github.io/maplibre-rust-ui/api/maplibre_rust_ui/component/struct.Component.html),
so an engine other than mln-rs, such as MapLibre Native or GL JS on the web, can stand in behind it
without the hosts knowing.

| Host | Crate | Runs on | Where the map draws |
|---|---|---|---|
| [Slint](slint.md) | `maplibre-slint` | macOS | A texture on Slint's wgpu device, which a Slint `Image` shows |
| [egui](egui.md) | `maplibre-egui` | macOS | A texture on egui's wgpu device, which egui shows as an image |
| [Bevy](bevy.md) | `maplibre-bevy` | macOS | A texture on Bevy's wgpu device, which a UI node shows |
| [GPUI](gpui.md) | `maplibre-gpui` | macOS | A ring of IOSurfaces, which GPUI's Metal renderer samples |
| [Native Shell](nsh.md) | `maplibre-nsh` | macOS, iOS | The `CAMetalLayer` of a native view of its own |
| [Whisker](whisker.md) | `maplibre-whisker` | iOS, Android | The `CAMetalLayer` of a Swift view, or a Kotlin `SurfaceView` |
| [Exact](exact.md) | `maplibre-exact` | macOS, iOS, Android | A texture on Exact's GPU canvas device, drawn into the canvas |

In no host is a frame copied on its way to the screen. The hosts that draw with wgpu lend the map a
texture on their own device, which they sample when they draw their frame. GPUI draws with Metal of its
own, so the map renders into IOSurfaces that GPUI samples. Native Shell and Whisker give the map a view
of its own, whose layer or surface takes the map's frames.

The map renders on a thread of its own in the hosts with native views and in GPUI, and before each of
the host's frames, on the host's thread, in the wgpu hosts and Exact. [Design](../design.md) has how
the engine draws.
