package org.maplibre.whisker

import android.content.Context
import android.view.Choreographer
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
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
 * on the choreographer while the map wants them, and posts the events each frame returns.
 */
class MapLibreSurfaceView(context: Context) : SurfaceView(context), SurfaceHolder.Callback, Choreographer.FrameCallback {
    private var map: Long = nativeNew()
    private var scheduled = false
    private var hasSurface = false

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
}
