package org.maplibre.whisker

import android.content.Context
import android.graphics.Matrix
import android.view.View
import android.view.ViewGroup
import rs.whisker.runtime.WhiskerContext
import rs.whisker.runtime.WhiskerUI

/**
 * A marker as Whisker's module view: it names the content's marker its node goes to, and the map moves that node
 * there after each frame (`MapLibreSurfaceView.placeMarkers`). The app's views are the node's children, so the node
 * is what moves, turns and tilts.
 */
class MapLibreMarkerView(context: WhiskerContext) : WhiskerUI<View>(context) {
    /** The content's marker this view's node goes to. */
    var marker = ""
        set(value) {
            field = value
            mapView()?.placeMarkers()
        }

    /** Whether the map hid the node, which lies off the map or behind the globe. */
    internal var hiddenByMap = false

    /** The animation matrix the map gave the node. */
    internal var placed: Matrix? = null

    override fun createView(context: Context) = View(context)

    /** The surface of the map whose children hold this marker. */
    private fun mapView(): MapLibreSurfaceView? {
        var ancestor = parent
        while (ancestor is ViewGroup) {
            for (index in 0 until ancestor.childCount) {
                (ancestor.getChildAt(index) as? MapLibreMapView)?.let { return it.view() }
            }
            ancestor = ancestor.parent
        }
        return null
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        super.onLayout(changed, left, top, right, bottom)
        mapView()?.placeMarkers()
    }
}
