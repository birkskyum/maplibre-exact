package org.maplibre.whisker

import rs.whisker.runtime.Module
import rs.whisker.runtime.ModuleDefinition
import rs.whisker.runtime.WhiskerModule

/** The map's module: the KSP processor registers its views under the names the Rust elements declare. */
@WhiskerModule
class MapLibreModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("MapLibre")
        View("maplibre:Map", MapLibreMapView::class.java) {
            Prop("map-style", clear = { view: MapLibreMapView -> view.setProp("map-style", "") }) { view: MapLibreMapView, value ->
                view.setProp("map-style", value.asString() ?: "")
            }
            Prop("camera", clear = { view: MapLibreMapView -> view.setProp("camera", "") }) { view: MapLibreMapView, value ->
                view.setProp("camera", value.asString() ?: "")
            }
            Prop("content", clear = { view: MapLibreMapView -> view.setProp("content", "") }) { view: MapLibreMapView, value ->
                view.setProp("content", value.asString() ?: "")
            }
            Prop("options", clear = { view: MapLibreMapView -> view.setProp("options", "") }) { view: MapLibreMapView, value ->
                view.setProp("options", value.asString() ?: "")
            }
            Command("command") { view: MapLibreMapView, value -> view.command(value.asString() ?: "") }
            Events("load", "moveend", "map_click", "contextmenu", "error")
        }
        View("maplibre:Marker", MapLibreMarkerView::class.java) {
            Prop("marker", clear = { view: MapLibreMarkerView -> view.marker = "" }) { view: MapLibreMarkerView, value ->
                view.marker = value.asString() ?: ""
            }
        }
    }
}
