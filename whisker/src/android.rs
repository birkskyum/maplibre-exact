//! The JNI functions the map's Kotlin view (`MapLibreSurfaceView`) calls, on
//! the main thread, over the same maps as the Swift view's C functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use jni::JNIEnv;
use jni::objects::{JFloatArray, JObject, JString};
use jni::sys::{jboolean, jdouble, jint, jlong, jstring};

use crate::ffi;

#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_fromSurface(env: *mut jni::sys::JNIEnv, surface: jni::sys::jobject) -> *mut c_void;
    fn ANativeWindow_release(window: *mut c_void);
}

thread_local! {
    /// The events each map's last frame posts, until the view takes them.
    static POSTS: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
}

fn id(map: jlong) -> u64 {
    u64::try_from(map).unwrap_or_default()
}

fn text(env: &mut JNIEnv, text: &JString) -> String {
    env.get_string(text).map(Into::into).unwrap_or_default()
}

/// A map, which wakes the view's frames through its `resume()`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeNew(env: JNIEnv, view: JObject) -> jlong {
    let (Ok(vm), Ok(view)) = (env.get_java_vm(), env.new_global_ref(view)) else { return 0 };
    let wake = Box::new(move || {
        if let Ok(mut env) = vm.attach_current_thread() {
            let _ = env.call_method(&view, "resume", "()V", &[]);
        }
    });
    jlong::try_from(ffi::insert(Some(wake))).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeDrop(_: JNIEnv, _: JObject, map: jlong) {
    POSTS.with(|posts| posts.borrow_mut().remove(&id(map)));
    ffi::maplibre_whisker_drop(id(map));
}

/// The view's `Surface`, or none once it is destroyed.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeSetSurface(
    env: JNIEnv,
    _: JObject,
    map: jlong,
    surface: JObject,
) {
    let window = if surface.is_null() {
        None
    } else {
        // SAFETY: the env and the `Surface` are live for the call.
        NonNull::new(unsafe { ANativeWindow_fromSurface(env.get_raw(), surface.as_raw()) })
    };
    // SAFETY: the reference `ANativeWindow_fromSurface` took.
    let release: fn(NonNull<c_void>) = |window| unsafe { ANativeWindow_release(window.as_ptr()) };
    ffi::set_surface(id(map), window.map(|window| (window, release)));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeSetProp(
    mut env: JNIEnv,
    _: JObject,
    map: jlong,
    name: JString,
    value: JString,
) {
    let (Ok(name), Ok(value)) =
        (std::ffi::CString::new(text(&mut env, &name)), std::ffi::CString::new(text(&mut env, &value)))
    else {
        return;
    };
    // SAFETY: both are NUL-terminated.
    unsafe { ffi::maplibre_whisker_set_prop(id(map), name.as_ptr(), value.as_ptr()) };
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeCommand(
    mut env: JNIEnv,
    _: JObject,
    map: jlong,
    json: JString,
) {
    let Ok(json) = std::ffi::CString::new(text(&mut env, &json)) else { return };
    // SAFETY: NUL-terminated.
    unsafe { ffi::maplibre_whisker_command(id(map), json.as_ptr()) };
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeResize(
    _: JNIEnv,
    _: JObject,
    map: jlong,
    width: jdouble,
    height: jdouble,
    scale: jdouble,
) {
    ffi::maplibre_whisker_resize(id(map), width, height, scale);
}

/// A touch: its pointer id, `phase` as the C function takes it, and where it
/// is in points.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativePointer(
    _: JNIEnv,
    _: JObject,
    map: jlong,
    pointer: jint,
    phase: jint,
    x: jdouble,
    y: jdouble,
) {
    ffi::maplibre_whisker_pointer(id(map), pointer.unsigned_abs(), phase, x, y, 0, true, 0);
}

/// Runs a frame. Returns whether the map wants another; the events it posts
/// wait for `nativeTakePosts`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeFrame(
    _: JNIEnv,
    _: JObject,
    map: jlong,
) -> jboolean {
    let (again, posts) = ffi::frame(id(map));
    if let Some(posts) = posts {
        POSTS.with(|pending| pending.borrow_mut().insert(id(map), posts));
    }
    jboolean::from(again)
}

/// The events of the last frame, as `maplibre_whisker_frame` returns them,
/// or null for none.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativeTakePosts(
    env: JNIEnv,
    _: JObject,
    map: jlong,
) -> jstring {
    let posts = POSTS.with(|pending| pending.borrow_mut().remove(&id(map)));
    posts.and_then(|posts| env.new_string(posts).ok()).map_or(std::ptr::null_mut(), JString::into_raw)
}

/// Where the content's marker `marker` puts a view laid out at `x`, `y`,
/// `width` by `height` points, as `maplibre_whisker_place` returns it, with
/// the homography written to `out`'s 9 floats.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_maplibre_whisker_MapLibreSurfaceView_nativePlace(
    mut env: JNIEnv,
    _: JObject,
    map: jlong,
    marker: JString,
    x: jdouble,
    y: jdouble,
    width: jdouble,
    height: jdouble,
    out: JFloatArray,
) -> jint {
    let marker = text(&mut env, &marker);
    let Some(placement) = ffi::place(id(map), &marker, [x, y, width, height].map(|n| n as f32)) else { return 0 };
    if placement.hidden {
        return 2;
    }
    if env.set_float_array_region(&out, 0, &placement.homography).is_err() {
        return 0;
    }
    1
}
