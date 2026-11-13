// A marker's view: it names the content's marker its node goes to, and the
// map moves that node there after each frame (`MapLibreLayerView.placeMarkers`).
// The app's views are the node's children, which Whisker lays out over this
// view, so the node is what moves, turns and tilts.

import Foundation
import QuartzCore
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif
import WhiskerModule

/// The marker as Whisker's module view.
@objc(MapLibreMarkerView)
public final class MapLibreMarkerView: WhiskerUI<MapLibrePlatformView> {
    /// The content's marker this view's node goes to.
    var marker = "" {
        didSet { mapLayerView()?.placeMarkers() }
    }

    /// Whether the map hid the node, which lies off the map or behind the globe.
    var hiddenByMap = false
    #if canImport(UIKit)
    /// Whether the node has the map's transform.
    var placed = false
    #else
    /// Where the map moved the node, and where the layout had put it.
    var placedOrigin: CGPoint?
    var layoutOrigin = CGPoint.zero
    #endif

    @objc public override func createView() -> MapLibrePlatformView {
        MapLibrePlatformView(frame: .zero)
    }

    /// The layer view of the map whose children hold this marker.
    func mapLayerView() -> MapLibreLayerView? {
        var ancestor = superview
        while let current = ancestor {
            for child in current.subviews {
                if let map = child as? MapLibreMapView { return map.view() }
            }
            ancestor = current.superview
        }
        return nil
    }

    #if canImport(UIKit)
    public override func layoutSubviews() {
        super.layoutSubviews()
        mapLayerView()?.placeMarkers()
    }
    #else
    public override func layout() {
        super.layout()
        mapLayerView()?.placeMarkers()
    }
    #endif
}
