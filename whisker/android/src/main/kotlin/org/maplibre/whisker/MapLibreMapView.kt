package org.maplibre.whisker

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Matrix
import android.view.Choreographer
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import org.json.JSONArray
import rs.whisker.runtime.WhiskerContext
import rs.whisker.runtime.WhiskerCustomEvent
import rs.whisker.runtime.WhiskerUI

/** The map as Whisker's module view. */
class MapLibreMapView(context: WhiskerContext) : WhiskerUI<MapLibreSurfaceView>(context) {
    init {
        view().post = { name, record -> WhiskerCustomEvent.dispatch(this, name, mapOf("record" to record)) }
    }

    override fun createView(context: Context) = MapLibreSurfaceView(context)

    fun setProp(name: String, value: String) = view().setProp(name, value)

    fun command(json: String) = view().command(json)
}

/**
 * The view the Rust map renders into, on a thread of its own, through its `Surface`. It hands the map its
 * props, size, touches and commands through the crate's JNI functions (`src/android.rs`), runs its frames
 * on the choreographer while the map wants them, posts the events each frame returns, and moves its
 * markers' nodes to their places.
 */
class MapLibreSurfaceView(context: Context) : SurfaceView(context), SurfaceHolder.Callback, Choreographer.FrameCallback {
    private var map: Long = nativeNew()
    private var scheduled = false
    private var hasSurface = false
    private val homography = FloatArray(9)

    /** Posts an event of the map, by its name, with its record as JSON. */
    var post: ((String, String) -> Unit)? = null

    init {
        holder.addCallback(this)
    }

    fun setProp(name: String, value: String) = nativeSetProp(map, name, value)

    fun command(json: String) = nativeCommand(map, json)

    /** Runs frames until the map wants none; the map calls it when it wants them again. */
    fun resume() {
        if (!scheduled && hasSurface && map != 0L) {
            scheduled = true
            Choreographer.getInstance().postFrameCallback(this)
        }
    }

    override fun doFrame(frameTimeNanos: Long) {
        scheduled = false
        if (map == 0L) return
        val again = nativeFrame(map)
        nativeTakePosts(map)?.let { posts ->
            val list = JSONArray(posts)
            for (i in 0 until list.length()) {
                val event = list.getJSONObject(i)
                post?.invoke(event.getString("name"), event.getString("record"))
            }
        }
        if (again) resume()
        placeMarkers()
    }

    /** Moves the nodes of the map's markers to where the content's markers put them, on the map as its last frame drew it. */
    fun placeMarkers() {
        val node = (parent as? View)?.parent as? ViewGroup ?: return
        markers(node).forEach { place(it, node) }
    }

    /** The markers among a node's children, but not those of a map inside it. */
    private fun markers(group: ViewGroup): List<MapLibreMarkerView> = (0 until group.childCount).flatMap { index ->
        when (val child = group.getChildAt(index)) {
            is MapLibreMapView -> emptyList()
            is MapLibreMarkerView -> listOf(child)
            is ViewGroup ->
                if ((0 until child.childCount).any { child.getChildAt(it) is MapLibreMapView }) emptyList() else markers(child)
            else -> emptyList()
        }
    }

    private fun place(marker: MapLibreMarkerView, mapNode: View) {
        val node = marker.parent as? View ?: return
        // Where the node's parent is in the map's node, and where the layout put the node in its parent.
        var offsetX = 0f
        var offsetY = 0f
        var ancestor = node.parent as? View
        while (ancestor != null && ancestor !== mapNode) {
            offsetX += ancestor.left + ancestor.translationX
            offsetY += ancestor.top + ancestor.translationY
            ancestor = ancestor.parent as? View
        }
        if (ancestor == null) return
        val x = node.left + node.translationX
        val y = node.top + node.translationY
        val density = resources.displayMetrics.density
        val placed = nativePlace(
            map, marker.marker, ((offsetX + x) / density).toDouble(), ((offsetY + y) / density).toDouble(),
            (node.width / density).toDouble(), (node.height / density).toDouble(), homography,
        )
        val hidden = placed == 2
        if (hidden != marker.hiddenByMap) {
            marker.hiddenByMap = hidden
            node.visibility = if (hidden) View.INVISIBLE else View.VISIBLE
        }
        // The homography maps the node's points to the map's, and a view's animation matrix maps its
        // laid-out pixels in its parent.
        val matrix = if (placed == 1) {
            Matrix().apply {
                setValues(homography)
                preScale(1 / density, 1 / density)
                postScale(density, density)
                preTranslate(-x, -y)
                postTranslate(-offsetX, -offsetY)
            }
        } else {
            null
        }
        if (matrix != marker.placed) {
            marker.placed = matrix
            setAnimationMatrix(node, matrix)
        }
    }

    // `setAnimationMatrix` is public from API 29 and on `RenderNode` since API 21, as Whisker's own
    // transforms use it.
    @SuppressLint("NewApi")
    private fun setAnimationMatrix(view: View, matrix: Matrix?) {
        try {
            view.setAnimationMatrix(matrix)
        } catch (_: NoSuchMethodError) {
            return
        }
        (view.parent as? View)?.invalidate()
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        hasSurface = true
        nativeSetSurface(map, holder.surface)
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        val density = resources.displayMetrics.density.toDouble()
        nativeResize(map, width / density, height / density, density)
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        hasSurface = false
        nativeSetSurface(map, null)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> pointer(event, event.actionIndex, 0)
            MotionEvent.ACTION_MOVE -> for (i in 0 until event.pointerCount) pointer(event, i, 1)
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> pointer(event, event.actionIndex, 2)
            MotionEvent.ACTION_CANCEL -> for (i in 0 until event.pointerCount) pointer(event, i, 3)
        }
        return true
    }

    private fun pointer(event: MotionEvent, index: Int, phase: Int) {
        val density = resources.displayMetrics.density.toDouble()
        nativePointer(map, event.getPointerId(index) + 1, phase, event.getX(index) / density, event.getY(index) / density)
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        Choreographer.getInstance().removeFrameCallback(this)
        scheduled = false
        nativeDrop(map)
        map = 0
    }

    private external fun nativeNew(): Long
    private external fun nativeDrop(map: Long)
    private external fun nativeSetSurface(map: Long, surface: android.view.Surface?)
    private external fun nativeSetProp(map: Long, name: String, value: String)
    private external fun nativeCommand(map: Long, json: String)
    private external fun nativeResize(map: Long, width: Double, height: Double, scale: Double)
    private external fun nativePointer(map: Long, pointer: Int, phase: Int, x: Double, y: Double)
    private external fun nativeFrame(map: Long): Boolean
    private external fun nativeTakePosts(map: Long): String?
    private external fun nativePlace(
        map: Long, marker: String, x: Double, y: Double, width: Double, height: Double, out: FloatArray,
    ): Int
}
