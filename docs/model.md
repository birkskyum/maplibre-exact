# The map model

An app declares its map with a base style, its own content over it, a camera and options. It sends
commands as JSON, hears of the events it listens for, and reads the map's record. Everything has
MapLibre GL JS's names and the MapLibre Style Specification's terms, and nothing names a UI framework
or an engine, so the same declaration works in every host.

The hosts take it in their own terms.

| Host | Declaration | Commands | Events |
|---|---|---|---|
| Slint, egui, Bevy, GPUI | An `Inputs` in Rust | A `Command` | `Event`s, with the record |
| Native Shell | The component's props, as JSON | `command(name, args)` on the element's ref | Native events, with the record |
| Whisker | The `Map` element's props, as JSON | A `MapHandle` | `on_load`, `on_map_click` and the others, with the record |
| Exact | The canvas's arguments | `postMessage` to the canvas | Messages to the canvas's `message=` action |

The core's [`model`](https://birkskyum.github.io/maplibre-rust-ui/api/maplibre_rust_ui/model/) module
has the types, which each host re-exports.

## Inputs

| Field | What it is |
|---|---|
| `style` | A style URL, or a style document as JSON |
| `camera` | Where the map looks, `{"center": [lng, lat], "zoom": z, "bearing": b, "pitch": p}` |
| `content` | The app's sources, layers, images and markers over the style |
| `options` | Where the camera may go and which gestures move it |
| `events` | The events the map posts to the app |

A new `style` switches the base style and keeps the content. The camera moves the map only when its
value changes, so the person can pan freely, and the app moves the map again by changing the value.
`@exact/maps` treats its camera props the same way.

## Content

The content is the app's own part of the style, in the style specification's terms, which the map
applies over the base style and follows as the app's state changes.

```json
{
  "sources": {
    "places": {"type": "geojson", "data": {"type": "FeatureCollection", "features": []}}
  },
  "layers": [
    {"id": "places", "type": "circle", "source": "places",
     "paint": {"circle-radius": 8, "circle-color": "#2f6fed"}},
    {"id": "place-names", "type": "symbol", "source": "places",
     "layout": {"text-field": ["get", "name"], "text-font": ["Noto Sans Bold"]}}
  ],
  "hitPadding": 6,
  "markers": {"station": {"lngLat": [8.5402, 47.3779], "anchor": "bottom"}},
  "projection": {"type": "globe"}
}
```

| Field | What it is |
|---|---|
| `sources` | Sources by id, as the style writes them |
| `layers` | Layers in drawing order, the last on top, as the style writes them, with `beforeId` or `beforeType` |
| `images` | Images that layers use, such as an `icon-image`, by name |
| `hitPadding` | How many points around a click still hit a feature, which makes small circles and thin lines easier to tap |
| `markers` | The app's own elements held at places on the map |
| `projection`, `sky`, `light`, `terrain` | The style's root properties, over the base style's |
| `state` | Values that the style's `global-state` expressions read, as GL JS's `setGlobalStateProperty` sets them |

### Layers

A layer draws above the base style and above the content's layers before it. With `beforeId` it draws
under the layer of that id, as GL JS's `addLayer` takes it. With `beforeType` it draws under the base
style's first layer of that type, so `"beforeType": "symbol"` draws it under the labels whatever the
style calls them. A layer may use a source of the base style.

When the content changes, the map does the least it can.

- A GeoJSON source takes new data in place.
- A layer takes changed paint and layout properties, filter and zoom range in place.
- A changed source type or tiles, a changed layer type or source, or a new order removes and adds those
  layers and sources again.

Content is applied once the style has loaded, and again after a new style loads, so switching base
styles keeps it.

### Images

`images` names each image by the name layers use. Its value is the image's path, or an object as GL
JS's `addImage` takes it.

```json
"images": {
  "pin": "assets/pin.png",
  "dot": {"src": "assets/dot@2x.png", "pixelRatio": 2, "sdf": false}
}
```

Exact loads each image from the app's `assets/` folder itself. In Slint, egui and GPUI the app hands
the map each image's file with `add_image(src, bytes)`. Native Shell, Whisker and Bevy don't take
images yet.

### Markers

`markers` holds the app's own elements at places on the map, as GL JS's `Marker` holds an element, by a
name the host matches to an element.

```json
"markers": {
  "station": {"lngLat": [8.5402, 47.3779], "anchor": "bottom", "offset": [0, -14]},
  "compass": {"rotationAlignment": "map"}
}
```

| Option | What it is |
|---|---|
| `lngLat` | `[longitude, latitude]` in degrees |
| `anchor` | The part of the element at the place, which it turns about: `center` (the default), `top`, `bottom`, `left`, `right`, `top-left`, `top-right`, `bottom-left` or `bottom-right` |
| `offset` | Points the element moves from there, `[right, down]` |
| `rotation` | Degrees clockwise |
| `rotationAlignment` | `map` turns the element with the map's bearing. `viewport` and `auto` keep it upright |
| `pitchAlignment` | `map` lays the element on the map as it pitches. `auto` follows `rotationAlignment` |

A marker whose place is off the map or behind the globe is hidden. A marker without `lngLat` stays
where the layout puts it and only turns, which makes a compass.

How far a marker follows the map depends on what the host's framework can do with an element.

| Host | Names the element by | Turns with the map | Lies on the map |
|---|---|---|---|
| Exact | Its `testId` | Yes | Yes |
| Whisker | `MapMarker(marker: …)` | Yes, on iOS and Android | Yes, on iOS and Android |
| Slint | `MapLibreMarker`'s `place.name` | Yes | No, it stays unsquashed |
| egui | `map.marker(ctx, name, …)` | No | No |
| Bevy | `MapMarker { id }` | No | No |
| GPUI | `at_marker` with a place's `name` | No | No |
| Native Shell | `MarkerPlace::of(data, name)` | No | No |

### Root properties and state

`projection`, `sky`, `light` and `terrain` set the style's root properties over the base style's,
which come back when the content drops them. `{"projection": {"type": "globe"}}` turns the globe on.
What an engine doesn't draw is reported in the record's `failure` and an `error` event, and doesn't
refuse the map, since another engine may draw it. mln-rs draws no 3D terrain, so `terrain` is
reported that way.

`state` sets the values that the style's `global-state` expressions read, by name.

## Options

Options limit the camera and turn gestures off, with GL JS's `MapOptions` names. Each is optional, and
none at all are GL JS's defaults. Clicks reach the app whatever the gestures do. A name the map doesn't
take is refused.

| Option | What it is |
|---|---|
| `minZoom`, `maxZoom` | The zoom range |
| `minPitch`, `maxPitch` | The pitch range, in degrees |
| `maxBounds` | `[[west, south], [east, north]]`, which the camera's center stays in |
| `interactive` | Whether gestures move the camera at all |
| `dragRotate` | Whether gestures turn the bearing |
| `pitchWithRotate` | Whether a drag with the rotating button pitches |
| `touchPitch` | Whether two fingers moving up or down together pitch |
| `scrollZoom` | Whether a wheel or a trackpad's scroll zooms |

The gestures are GL JS's, from mln-rs's gesture recognizer. A drag pans, a wheel or a pinch zooms, a
double click zooms in, a right drag or a ctrl-drag turns and pitches, a shift-drag zooms to a box, two
fingers turn and pitch, and the arrow keys and `+` and `-` move the map.

## Commands

A command is a JSON object whose `command` names it, with GL JS's camera options.

```json
{"command": "flyTo", "center": [8.54, 47.37], "zoom": 14, "duration": 2000}
```

| Command | What it does |
|---|---|
| `jumpTo` | Moves the camera at once |
| `easeTo` | Moves the camera in an animation |
| `flyTo` | Moves the camera in an animation that zooms out and in on the way, as GL JS's does |
| `fitBounds` | Shows the box between two corners, `{"bounds": [[west, south], [east, north]], "padding": 40}`, animated unless `duration` is 0 |
| `setStyle` | Switches the base style, `{"style": url}`, keeping the content |

`jumpTo`, `easeTo` and `flyTo` take `center`, `zoom`, `bearing` and `pitch`, and set only those given.
`easeTo` and `flyTo` take a `duration` in milliseconds, which is the engine's own without one. Each of
them can also take `padding`, the points of the map's edges that the camera's center keeps clear of,
such as under a panel over the map. It is one number for every edge, or each edge's own,
`{"top": 0, "bottom": 240, "left": 12, "right": 0}`, and it stays until a command changes it.

## Events

The map posts the events `events` names, as GL JS's `map.on` subscribes to them. Each comes with the
map's record, which has the details.

| Event | When |
|---|---|
| `load` | The style loaded and the map drew it, and the record has the sources' attribution |
| `moveend` | The camera came to rest, whoever moved it |
| `click` | A press and release that didn't move the map, which the record's `click` describes |
| `contextmenu` | A secondary click, or a touch held still for half a second, which the record's `contextmenu` describes. Its release is no click |
| `error` | Something failed, which the record's `failure` says |

## The record

The record is what the map knows, which the app reads at any time. Its camera is the one at rest, so
the record changes when something happens and not on every frame of a gesture.

```json
{
  "center": [8.5402, 47.3779], "zoom": 14, "bearing": 0, "pitch": 0,
  "loaded": true,
  "click": {
    "lngLat": {"lng": 8.5402, "lat": 47.3779},
    "point": {"x": 400, "y": 300},
    "features": [
      {"id": null, "layer": "places", "source": "places", "sourceLayer": "",
       "properties": {"name": "Zürich HB"}, "clusterExpansionZoom": null}
    ]
  },
  "contextmenu": null,
  "failure": null,
  "attribution": "OpenFreeMap © OpenMapTiles Data from OpenStreetMap"
}
```

| Field | What it is |
|---|---|
| `center`, `zoom`, `bearing`, `pitch` | The camera where it last came to rest |
| `loaded` | Whether the style and every tile in view have loaded |
| `click` | The last click, `null` before the first |
| `contextmenu` | The last secondary click or long press, `null` before the first |
| `failure` | The last error, `null` while there has been none |
| `attribution` | Who the style's data comes from, as text to show on or beside the map, the sources' attributions joined by vertical bars |

A click has its `lngLat` and `point` as GL JS's `MapMouseEvent` has them, the point in points from the
map's top-left corner. Its `features` are those of the content's layers at the click, widened by
`hitPadding`, topmost first, as GL JS's `queryRenderedFeatures` returns them without their geometry. A
cluster of a clustered GeoJSON source carries `clusterExpansionZoom`, the zoom at which it splits, as
GL JS's `getClusterExpansionZoom` answers, so the app can zoom in to split it with one command.

The last error is called `failure` rather than `error`, because Exact keeps a canvas's own `error`
beside its record.
