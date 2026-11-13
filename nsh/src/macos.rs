//! The map on macOS: a view whose layer is a `CAMetalLayer`, which the map
//! renders into on a thread of its own, and which takes the mouse and the
//! trackpad itself.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use maplibre_rust_ui::gestures::{Button, Input, Modifiers, PinchPhase, WheelDelta};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::size::Size;
use maplibre_rust_ui::{Phase, Viewport};
use nsh_core::JsonValue;
use nsh_platform_macos::{ComponentContext, Components, NativeComponent};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventPhase, NSResponder, NSView};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSTimer};
use objc2_quartz_core::{CALayer, CAMetalLayer};

use maplibre_rust_ui::component::{self, Component, Press};

use crate::NAME;

/// Registers the map, `maplibre.map`, on `components`: the app calls it from
/// its `#[native_components(macos)]` function, or before `run_app` through
/// `nsh_platform_macos::set_components`.
pub fn register(components: &Components) {
    components.register(
        NAME,
        Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> {
            let mtm = MainThreadMarker::new().expect("views are made on the main thread");
            Box::new(MapComponent::new(context, mtm))
        }),
    );
}

/// What the component and its view share: the map, the layer it renders
/// into, and the timer that runs its frames while it wants them.
struct Shared {
    state: RefCell<Component>,
    layer: Retained<CAMetalLayer>,
    context: ComponentContext,
    timer: RefCell<Option<Retained<NSTimer>>>,
}

impl Shared {
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
        // whose common modes keep it firing while a window resizes or a
        // menu tracks, is the main thread's, where the view lives.
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
            self.context.emit(post.name, post.data);
        }
        if !again && let Some(timer) = self.timer.borrow_mut().take() {
            timer.invalidate();
        }
    }

    /// The view is `size` points at `scale` now.
    fn resize(self: &Rc<Shared>, size: NSSize, scale: f64) {
        self.layer.setContentsScale(scale);
        let viewport = (size.width >= 1.0 && size.height >= 1.0).then(|| Viewport {
            size: Size::new(size.width.round() as u32, size.height.round() as u32),
            scale: scale as f32,
        });
        self.state.borrow_mut().resize(viewport);
        self.wake();
    }
}

struct MapComponent {
    view: Retained<MapView>,
    shared: Rc<Shared>,
}

impl MapComponent {
    fn new(context: ComponentContext, mtm: MainThreadMarker) -> MapComponent {
        let layer = CAMetalLayer::new();
        layer.setOpaque(true);
        let shared =
            Rc::new(Shared { state: RefCell::new(Component::new()), layer, context, timer: RefCell::new(None) });
        let view = MapView::alloc(mtm).set_ivars(Rc::downgrade(&shared));
        // SAFETY: NSView's designated initializer.
        let view: Retained<MapView> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
        view.setWantsLayer(true);
        MapComponent { view, shared }
    }
}

impl NativeComponent for MapComponent {
    fn view(&self) -> Retained<NSView> {
        Retained::into_super(self.view.clone())
    }

    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        self.shared.state.borrow_mut().set_prop(name, value);
        self.shared.wake();
    }

    fn command(&mut self, name: &str, args: &JsonValue) {
        self.shared.state.borrow_mut().command(name, args);
        self.shared.wake();
    }
}

impl Drop for MapComponent {
    fn drop(&mut self) {
        if let Some(timer) = self.shared.timer.borrow_mut().take() {
            timer.invalidate();
        }
    }
}

define_class!(
    /// The map's view: its layer is the map's `CAMetalLayer`, and it hands
    /// the map its size and its input.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Weak<Shared>]
    #[name = "MapLibreNshMapView"]
    struct MapView;

    unsafe impl NSObjectProtocol for MapView {}

    impl MapView {
        #[unsafe(method_id(makeBackingLayer))]
        fn make_backing_layer(&self) -> Retained<CALayer> {
            match self.ivars().upgrade() {
                Some(shared) => Retained::into_super(shared.layer.clone()),
                None => CALayer::new(),
            }
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: NSView's own, which lays the view out.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.resized();
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            self.resized();
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            self.resized();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.press(event, Phase::Down, Button::Primary);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.press(event, Phase::Move, Button::Primary);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.press(event, Phase::Up, Button::Primary);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.press(event, Phase::Down, Button::Secondary);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.press(event, Phase::Move, Button::Secondary);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.press(event, Phase::Up, Button::Secondary);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.press(event, Phase::Down, Button::Middle);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.press(event, Phase::Move, Button::Middle);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.press(event, Phase::Up, Button::Middle);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // The web's sign, which the gestures take: down is positive.
            let delta = -event.scrollingDeltaY();
            let delta =
                if event.hasPreciseScrollingDeltas() { WheelDelta::Pixels(delta) } else { WheelDelta::Notches(delta) };
            let input = Input::Wheel { position: self.position(event), delta, modifiers: modifiers(event) };
            self.gesture(input);
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify_with_event(&self, event: &NSEvent) {
            let input = Input::Pinch {
                center: self.position(event),
                scale: 1.0 + event.magnification(),
                rotation: 0.0,
                phase: pinch_phase(event),
            };
            self.gesture(input);
        }

        #[unsafe(method(rotateWithEvent:))]
        fn rotate_with_event(&self, event: &NSEvent) {
            // AppKit turns counterclockwise, the gestures clockwise.
            let input = Input::Pinch {
                center: self.position(event),
                scale: 1.0,
                rotation: -f64::from(event.rotation()),
                phase: pinch_phase(event),
            };
            self.gesture(input);
        }
    }
);

impl MapView {
    fn resized(&self) {
        let Some(shared) = self.ivars().upgrade() else { return };
        let scale = self.window().map_or(2.0, |window| window.backingScaleFactor());
        shared.resize(self.frame().size, scale);
    }

    /// Where `event` is, in points from the view's top-left corner.
    fn position(&self, event: &NSEvent) -> ScreenCoordinate {
        let NSPoint { x, y } = self.convertPoint_fromView(event.locationInWindow(), None);
        ScreenCoordinate::new(x, y)
    }

    fn press(&self, event: &NSEvent, phase: Phase, button: Button) {
        let Some(shared) = self.ivars().upgrade() else { return };
        let ScreenCoordinate { x, y } = self.position(event);
        let press = Press { id: 0, phase, x, y, button, touch: false, modifiers: modifiers(event) };
        shared.state.borrow_mut().pointer(&press);
        shared.wake();
    }

    fn gesture(&self, input: Input) {
        let Some(shared) = self.ivars().upgrade() else { return };
        shared.state.borrow_mut().gesture(input);
        shared.wake();
    }
}

fn modifiers(event: &NSEvent) -> Modifiers {
    let flags = event.modifierFlags();
    Modifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        ctrl: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        meta: flags.contains(NSEventModifierFlags::Command),
    }
}

fn pinch_phase(event: &NSEvent) -> PinchPhase {
    let phase = event.phase();
    if phase.contains(NSEventPhase::Began) {
        PinchPhase::Begin
    } else if phase.contains(NSEventPhase::Ended) {
        PinchPhase::End
    } else if phase.contains(NSEventPhase::Cancelled) {
        PinchPhase::Cancel
    } else {
        PinchPhase::Update
    }
}
