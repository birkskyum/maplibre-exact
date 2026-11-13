//! The map on macOS: a view whose layer is a `CAMetalLayer`, which the map
//! renders into on a thread of its own, and which takes the mouse and the
//! trackpad itself.

use std::rc::{Rc, Weak};

use maplibre_rust_ui::Phase;
use maplibre_rust_ui::component::Press;
use maplibre_rust_ui::gestures::{Button, Input, Modifiers, PinchPhase, WheelDelta};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use nsh_core::JsonValue;
use nsh_platform_macos::{ComponentContext, Components, NativeComponent};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventPhase, NSResponder, NSView};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAMetalLayer};

use crate::NAME;
use crate::apple::Shared;

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

struct MapComponent {
    view: Retained<MapView>,
    shared: Rc<Shared>,
}

impl MapComponent {
    fn new(context: ComponentContext, mtm: MainThreadMarker) -> MapComponent {
        let shared = Shared::new(CAMetalLayer::new(), move |name, data| context.emit(name, data));
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
        self.shared.set_prop(name, value);
    }

    fn command(&mut self, name: &str, args: &JsonValue) {
        self.shared.command(name, args);
    }
}

impl Drop for MapComponent {
    fn drop(&mut self) {
        self.shared.stop();
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
        let size = self.frame().size;
        shared.resize(size.width, size.height, scale);
    }

    /// Where `event` is, in points from the view's top-left corner.
    fn position(&self, event: &NSEvent) -> ScreenCoordinate {
        let NSPoint { x, y } = self.convertPoint_fromView(event.locationInWindow(), None);
        ScreenCoordinate::new(x, y)
    }

    fn press(&self, event: &NSEvent, phase: Phase, button: Button) {
        let Some(shared) = self.ivars().upgrade() else { return };
        let ScreenCoordinate { x, y } = self.position(event);
        shared.press(&Press { id: 0, phase, x, y, button, touch: false, modifiers: modifiers(event) });
    }

    fn gesture(&self, input: Input) {
        let Some(shared) = self.ivars().upgrade() else { return };
        shared.gesture(input);
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
