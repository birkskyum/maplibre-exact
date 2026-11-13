//! The C functions the map's Swift view calls, on the main thread: it makes
//! a map of its `CAMetalLayer`, hands it its props, size, input and commands,
//! runs its frames while it wants them, and drops it. Each map has the id
//! [`maplibre_whisker_new`] returned, and an unknown id does nothing. A map
//! wakes its view when it wants frames again, whoever handed it something.
//! The Kotlin view on Android reaches the same maps through JNI.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr::NonNull;

use maplibre_rust_ui::component::{Component, Press};
use maplibre_rust_ui::runtime::{Driver, RenderTarget};
use maplibre_rust_ui::{Phase, Viewport};
use maplibre_rust_ui::gestures::{Button, Input, Modifiers, PinchPhase, WheelDelta};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::size::Size;
use serde_json::{Value as Json, json};

/// What wakes a Swift view's frames: a function and the view it is given.
pub type Wake = extern "C" fn(*mut c_void);

/// A view's `CAMetalLayer` or `ANativeWindow`, and what lets go of it.
pub(crate) type ViewSurface = (NonNull<c_void>, fn(NonNull<c_void>));

struct Map {
    component: Component,
    surface: Option<ViewSurface>,
    wake: Option<Box<dyn Fn()>>,
}

impl Drop for Map {
    fn drop(&mut self) {
        if let Some((surface, release)) = self.surface.take() {
            release(surface);
        }
    }
}

/// A target of the view's surface, which the map renders into.
fn target(surface: Option<NonNull<c_void>>, viewport: Viewport) -> Result<(RenderTarget, Driver), String> {
    let surface = surface.ok_or_else(|| "maplibre: the view has no surface".to_owned())?;
    #[cfg(target_vendor = "apple")]
    // SAFETY: the view keeps its `CAMetalLayer` while the map lives.
    return unsafe { maplibre_rust_ui::component::metal_layer_target(surface, viewport) };
    #[cfg(target_os = "android")]
    // SAFETY: the map holds a reference to the window.
    return unsafe { maplibre_rust_ui::component::android_window_target(surface, viewport) };
    #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
    {
        let _ = (surface, viewport);
        Err("maplibre: no native view on this platform".to_owned())
    }
}

/// A map, whose view wakes its frames with `wake`. Returns its id.
pub(crate) fn insert(wake: Option<Box<dyn Fn()>>) -> u64 {
    let id = NEXT.with(|next| next.replace(next.get() + 1));
    MAPS.with(|maps| maps.borrow_mut().insert(id, Map { component: Component::new(), surface: None, wake }));
    id
}

/// The map's view has `surface` now, which `release` lets go of, or none.
pub(crate) fn set_surface(map: u64, surface: Option<ViewSurface>) {
    let old = with_map(map, |map| std::mem::replace(&mut map.surface, surface));
    if let Some(Some((surface, release))) = old {
        release(surface);
    }
}

/// Runs a frame of the map. Returns whether it wants another, and the events
/// to post, `[{"name": "load", "record": "{…}"}, …]` with each record as JSON
/// text, or none.
pub(crate) fn frame(map: u64) -> (bool, Option<String>) {
    let framed = with_map(map, |map| {
        let surface = map.surface.map(|(surface, _)| surface);
        map.component.frame(|viewport| target(surface, viewport))
    });
    let (again, posts) = framed.unwrap_or_default();
    if posts.is_empty() {
        return (again, None);
    }
    let posts: Vec<Json> = posts
        .into_iter()
        .map(|post| {
            // Whisker keeps `click` for its own gesture.
            let name = if post.name == "click" { "map_click" } else { post.name };
            json!({"name": name, "record": post.data.to_string()})
        })
        .collect();
    (again, Some(Json::from(posts).to_string()))
}

thread_local! {
    static MAPS: RefCell<HashMap<u64, Map>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

fn with_map<T>(map: u64, f: impl FnOnce(&mut Map) -> T) -> Option<T> {
    MAPS.with(|maps| maps.borrow_mut().get_mut(&map).map(f))
}

/// Hands the map something, then wakes its view's frames.
fn hand(map: u64, f: impl FnOnce(&mut Component)) {
    // The view runs its frames when woken, outside the maps' borrow.
    let wake = with_map(map, |map| {
        f(&mut map.component);
        map.wake.take()
    });
    if let Some(Some(wake)) = wake {
        wake();
        with_map(map, |map| map.wake = Some(wake));
    }
}

/// The map's size in points, once its view has one.
#[must_use]
pub fn viewport(map: u64) -> Option<(f64, f64)> {
    let viewport = with_map(map, |map| map.component.viewport()).flatten()?;
    Some((f64::from(viewport.size.width), f64::from(viewport.size.height)))
}

/// The text at `text`, empty for null or text that isn't UTF-8.
///
/// # Safety
///
/// `text` is null or a NUL-terminated string.
unsafe fn text<'a>(text: *const c_char) -> &'a str {
    if text.is_null() {
        return "";
    }
    // SAFETY: the caller hands a NUL-terminated string.
    unsafe { CStr::from_ptr(text) }.to_str().unwrap_or_default()
}

fn modifiers(bits: u32) -> Modifiers {
    Modifiers { shift: bits & 1 != 0, ctrl: bits & 2 != 0, alt: bits & 4 != 0, meta: bits & 8 != 0 }
}

/// A map that renders into `layer`, a `CAMetalLayer`, once it has a size
/// and a style, and calls `wake` with `view` when it wants frames again.
/// Returns its id, or 0 for a null layer.
///
/// # Safety
///
/// `layer` is null or a `CAMetalLayer`, and `wake` takes `view`, both while
/// the map lives.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maplibre_whisker_new(layer: *mut c_void, wake: Option<Wake>, view: *mut c_void) -> u64 {
    let Some(layer) = NonNull::new(layer) else { return 0 };
    let view = view as usize;
    let id = insert(wake.map(|wake| Box::new(move || wake(view as *mut c_void)) as Box<dyn Fn()>));
    set_surface(id, Some((layer, |_| {})));
    id
}

/// Lets go of the map.
#[unsafe(no_mangle)]
pub extern "C" fn maplibre_whisker_drop(map: u64) {
    let dropped = MAPS.with(|maps| maps.borrow_mut().remove(&map));
    drop(dropped);
}

/// Prop `name` is `value`: `map-style`, a style URL or document, or
/// `camera`, `content` or `options` as JSON, which empty text removes.
///
/// # Safety
///
/// `name` and `value` are null or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maplibre_whisker_set_prop(map: u64, name: *const c_char, value: *const c_char) {
    // SAFETY: the caller hands NUL-terminated strings.
    let (name, value) = unsafe { (text(name), text(value)) };
    let value = match name {
        "map-style" => Some(Json::from(value)),
        _ if value.trim().is_empty() => None,
        // Text that isn't JSON fails as the prop's own, malformed.
        _ => Some(serde_json::from_str(value).unwrap_or_else(|_| Json::from(value))),
    };
    let name = if name == "map-style" { "style" } else { name };
    hand(map, |component| component.set_prop(name, value.as_ref()));
}

/// Sends the map the command `json`, `{"command": "flyTo", …}` with GL JS's
/// options.
///
/// # Safety
///
/// `json` is null or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maplibre_whisker_command(map: u64, json: *const c_char) {
    // SAFETY: the caller hands a NUL-terminated string.
    let mut command: Json = serde_json::from_str(unsafe { text(json) }).unwrap_or_else(|_| json!({}));
    let name = command.get("command").and_then(Json::as_str).unwrap_or_default().to_owned();
    if let Some(fields) = command.as_object_mut() {
        fields.remove("command");
    }
    hand(map, |component| component.command(&name, &command));
}

/// The view is `width` by `height` points at `scale` pixels a point.
#[unsafe(no_mangle)]
pub extern "C" fn maplibre_whisker_resize(map: u64, width: f64, height: f64, scale: f64) {
    let viewport = (width >= 1.0 && height >= 1.0)
        .then(|| Viewport { size: Size::new(width.round() as u32, height.round() as u32), scale: scale as f32 });
    hand(map, |component| component.resize(viewport));
}

/// A pointer on the view, at `x`, `y` points from its top-left corner:
/// `phase` 0 down, 1 move, 2 up, 3 cancel; `button` 0 the primary, 1 the
/// middle, 2 the secondary; `modifiers` 1 shift, 2 control, 4 alt, 8 meta.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn maplibre_whisker_pointer(
    map: u64,
    id: u32,
    phase: i32,
    x: f64,
    y: f64,
    button: i32,
    touch: bool,
    modifiers_bits: u32,
) {
    let phase = match phase {
        0 => Phase::Down,
        1 => Phase::Move,
        2 => Phase::Up,
        _ => Phase::Cancel,
    };
    let button = match button {
        1 => Button::Middle,
        2 => Button::Secondary,
        _ => Button::Primary,
    };
    let press = Press { id, phase, x, y, button, touch, modifiers: modifiers(modifiers_bits) };
    hand(map, |component| component.pointer(&press));
}

/// A wheel or trackpad scroll at `x`, `y`, `delta` by the web's sign (down
/// is positive), in points when `precise`, in a wheel's notches otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn maplibre_whisker_wheel(map: u64, x: f64, y: f64, delta: f64, precise: bool, modifiers_bits: u32) {
    let delta = if precise { WheelDelta::Pixels(delta) } else { WheelDelta::Notches(delta) };
    let input = Input::Wheel { position: ScreenCoordinate::new(x, y), delta, modifiers: modifiers(modifiers_bits) };
    hand(map, |component| component.gesture(input));
}

/// A pinch the platform recognized, such as a trackpad's, around `x`, `y`:
/// each zooms by `scale` and turns `rotation` degrees clockwise since the
/// last; `phase` 0 begin, 1 update, 2 end, 3 cancel.
#[unsafe(no_mangle)]
pub extern "C" fn maplibre_whisker_pinch(map: u64, x: f64, y: f64, scale: f64, rotation: f64, phase: i32) {
    let phase = match phase {
        0 => PinchPhase::Begin,
        2 => PinchPhase::End,
        3 => PinchPhase::Cancel,
        _ => PinchPhase::Update,
    };
    let input = Input::Pinch { center: ScreenCoordinate::new(x, y), scale, rotation, phase };
    hand(map, |component| component.gesture(input));
}

/// Runs a frame, and sets `again` to whether the map wants another. Returns
/// the events to post, `[{"name": "load", "record": "{…}"}, …]` with each
/// record as JSON text, which [`maplibre_whisker_free`] frees, or null for
/// none.
///
/// # Safety
///
/// `again` is null or points to a `bool`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maplibre_whisker_frame(map: u64, again: *mut bool) -> *mut c_char {
    let (wants, posts) = frame(map);
    if !again.is_null() {
        // SAFETY: the caller hands a `bool`.
        unsafe { *again = wants };
    }
    posts.and_then(|posts| CString::new(posts).ok()).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Frees what [`maplibre_whisker_frame`] returned.
///
/// # Safety
///
/// `text` is null or what [`maplibre_whisker_frame`] returned, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maplibre_whisker_free(text: *mut c_char) {
    if !text.is_null() {
        // SAFETY: the text came from `CString::into_raw`.
        drop(unsafe { CString::from_raw(text) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_takes_props_and_posts_what_failed() {
        let mut layer = 0_u8;
        // SAFETY: the map never renders without a size, so the layer is never read.
        let map = unsafe { maplibre_whisker_new((&raw mut layer).cast(), None, std::ptr::null_mut()) };
        assert_ne!(map, 0);
        let set = |name: &CStr, value: &CStr| unsafe { maplibre_whisker_set_prop(map, name.as_ptr(), value.as_ptr()) };
        set(c"map-style", c"https://tiles.openfreemap.org/styles/liberty");
        set(c"camera", c"{\"zoom\": \"near\"}");
        let mut again = true;
        // SAFETY: `again` is a bool.
        let posts = unsafe { maplibre_whisker_frame(map, &raw mut again) };
        assert!(!again);
        // SAFETY: the frame returned NUL-terminated text.
        let text = unsafe { CStr::from_ptr(posts) }.to_str().unwrap().to_owned();
        unsafe { maplibre_whisker_free(posts) };
        let posts: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(posts[0]["name"], "error");
        let record: Json = serde_json::from_str(posts[0]["record"].as_str().unwrap()).unwrap();
        assert_eq!(record["failure"], "maplibre: camera: zoom is a number");
        maplibre_whisker_drop(map);
        // SAFETY: a dropped map's id does nothing.
        assert!(unsafe { maplibre_whisker_frame(map, std::ptr::null_mut()) }.is_null());
    }
}
