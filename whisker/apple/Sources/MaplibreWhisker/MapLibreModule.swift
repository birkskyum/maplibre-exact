// The map's module: Whisker's codegen plugin registers its view under the
// name the Rust element `maplibre_whisker::Map` declares.

import Foundation
import WhiskerModule

@WhiskerModule
public final class MapLibreModule: Module {
    public override func definition() -> ModuleDefinition {
        ModuleDefinition {
            Name("MapLibre")
            View("maplibre:Map", MapLibreMapView.self) {
                Prop("map-style") { (view: MapLibreMapView, value: WhiskerValue) in
                    view.setProp("map-style", value.asString ?? "")
                }
                Prop("camera") { (view: MapLibreMapView, value: WhiskerValue) in
                    view.setProp("camera", value.asString ?? "")
                }
                Prop("content") { (view: MapLibreMapView, value: WhiskerValue) in
                    view.setProp("content", value.asString ?? "")
                }
                Prop("options") { (view: MapLibreMapView, value: WhiskerValue) in
                    view.setProp("options", value.asString ?? "")
                }
                Command("command") { (view: MapLibreMapView, value: WhiskerValue) in
                    view.command(value.asString ?? "")
                }
                Events("load", "moveend", "map_click", "contextmenu", "error")
            }
        }
    }
}
