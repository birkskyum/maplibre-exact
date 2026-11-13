//! What the map's views on Apple's platforms share: the map, the
//! `CAMetalLayer` it renders into on a thread of its own, and the timer on
//! the main run loop that runs its frames while it wants them.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use maplibre_rust_ui::Viewport;
use maplibre_rust_ui::component::{self, Component, Press};
use maplibre_rust_ui::gestures::Input;
use maplibre_rust_ui::util::size::Size;
use nsh_core::JsonValue;
use objc2::rc::Retained;
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
use objc2_quartz_core::CAMetalLayer;

/// Posts an event to the app, by its name, with its data.
type Emit = Box<dyn Fn(&str, JsonValue)>;

pub(crate) struct Shared {
    state: RefCell<Component>,
    pub layer: Retained<CAMetalLayer>,
    emit: Emit,
    timer: RefCell<Option<Retained<NSTimer>>>,
}

impl Shared {
    pub fn new(layer: Retained<CAMetalLayer>, emit: impl Fn(&str, JsonValue) + 'static) -> Rc<Shared> {
        layer.setOpaque(true);
        Rc::new(Shared {
            state: RefCell::new(Component::new()),
            layer,
            emit: Box::new(emit),
            timer: RefCell::new(None),
        })
    }

    pub fn set_prop(self: &Rc<Shared>, name: &str, value: Option<&JsonValue>) {
        self.state.borrow_mut().set_prop(name, value);
        self.wake();
    }

    pub fn command(self: &Rc<Shared>, name: &str, args: &JsonValue) {
        self.state.borrow_mut().command(name, args);
        self.wake();
    }

    pub fn press(self: &Rc<Shared>, press: &Press) {
        self.state.borrow_mut().pointer(press);
        self.wake();
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn gesture(self: &Rc<Shared>, input: Input) {
        self.state.borrow_mut().gesture(input);
        self.wake();
    }

    /// The view is `width` by `height` points at `scale` pixels a point.
    pub fn resize(self: &Rc<Shared>, width: f64, height: f64, scale: f64) {
        self.layer.setContentsScale(scale);
        let viewport = (width >= 1.0 && height >= 1.0)
            .then(|| Viewport { size: Size::new(width.round() as u32, height.round() as u32), scale: scale as f32 });
        self.state.borrow_mut().resize(viewport);
        self.wake();
    }

    /// Runs frames until the map wants none.
    fn wake(self: &Rc<Shared>) {
        if self.timer.borrow().is_some() {
            return;
        }
        let shared = Rc::downgrade(self);
        let block = RcBlock::new(move |_: NonNull<NSTimer>| {
            if let Some(shared) = shared.upgrade() {
                shared.frame();
            }
        });
        // SAFETY: the block takes the timer it is given, and the run loop,
        // whose common modes keep it firing while a window resizes, a menu
        // tracks or a scroll view scrolls, is the main thread's, where the
        // view lives.
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(1.0 / 60.0, true, &block) };
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        *self.timer.borrow_mut() = Some(timer);
    }

    fn frame(&self) {
        let layer = NonNull::from(&*self.layer).cast::<c_void>();
        // SAFETY: the layer is a `CAMetalLayer`, which this keeps.
        let (again, posts) =
            self.state.borrow_mut().frame(|viewport| unsafe { component::metal_layer_target(layer, viewport) });
        for post in posts {
            (self.emit)(post.name, post.data);
        }
        if !again {
            self.stop();
        }
    }

    /// Stops the frames, as when the component goes.
    pub fn stop(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.invalidate();
        }
    }
}
