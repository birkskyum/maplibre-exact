//! The map on iOS: a view whose layer is a `CAMetalLayer`, which the map
//! renders into on a thread of its own, and which takes its touches itself.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use maplibre_rust_ui::Phase;
use maplibre_rust_ui::component::Press;
use maplibre_rust_ui::gestures::{Button, Modifiers};
use nsh_core::JsonValue;
use nsh_platform_ios::{ComponentContext, Components, NativeComponent};
use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSObject, NSObjectProtocol, NSSet};
use objc2_quartz_core::CAMetalLayer;
use objc2_ui_kit::{UIEvent, UIResponder, UITouch, UITraitEnvironment, UIView};

use crate::NAME;
use crate::apple::Shared;

/// Registers the map, `maplibre.map`, on `components`: the app calls it from
/// its `#[native_components(ios)]` function, or before `run_app` through
/// `nsh_platform_ios::set_components`.
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
        let view = MapView::alloc(mtm).set_ivars(Ivars::default());
        // SAFETY: UIView's designated initializer.
        let view: Retained<MapView> = unsafe { msg_send![super(view), initWithFrame: CGRect::default()] };
        view.setMultipleTouchEnabled(true);
        // SAFETY: the view's `layerClass` is `CAMetalLayer`.
        let layer = unsafe { Retained::cast_unchecked::<CAMetalLayer>(view.layer()) };
        let shared = Shared::new(layer, move |name, data| context.emit(name, data));
        *view.ivars().shared.borrow_mut() = Rc::downgrade(&shared);
        MapComponent { view, shared }
    }
}

impl NativeComponent for MapComponent {
    fn view(&self) -> &UIView {
        &self.view
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

#[derive(Default)]
struct Ivars {
    shared: RefCell<Weak<Shared>>,
    /// The pointer id of each touch down on the view, by the touch's address.
    touches: RefCell<HashMap<usize, u32>>,
    next: Cell<u32>,
}

define_class!(
    /// The map's view: its layer is the map's `CAMetalLayer`, and it hands
    /// the map its size and its touches.
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    #[name = "MapLibreNshMapView"]
    struct MapView;

    unsafe impl NSObjectProtocol for MapView {}

    impl MapView {
        #[unsafe(method(layerClass))]
        fn layer_class() -> &'static AnyClass {
            CAMetalLayer::class()
        }

        #[unsafe(method(layoutSubviews))]
        fn layout_subviews(&self) {
            // SAFETY: UIView's own.
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            self.resized();
        }

        #[unsafe(method(didMoveToWindow))]
        fn did_move_to_window(&self) {
            // SAFETY: UIView's own.
            let _: () = unsafe { msg_send![super(self), didMoveToWindow] };
            self.resized();
        }

        #[unsafe(method(touchesBegan:withEvent:))]
        fn touches_began(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.touches(touches, Phase::Down);
        }

        #[unsafe(method(touchesMoved:withEvent:))]
        fn touches_moved(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.touches(touches, Phase::Move);
        }

        #[unsafe(method(touchesEnded:withEvent:))]
        fn touches_ended(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.touches(touches, Phase::Up);
        }

        #[unsafe(method(touchesCancelled:withEvent:))]
        fn touches_cancelled(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.touches(touches, Phase::Cancel);
        }
    }
);

impl MapView {
    fn resized(&self) {
        let Some(shared) = self.ivars().shared.borrow().upgrade() else { return };
        // SAFETY: a property of the view's own trait collection, read on the main thread.
        let scale = unsafe { self.traitCollection().displayScale() };
        let size = self.bounds().size;
        shared.resize(size.width, size.height, if scale > 0.0 { scale } else { 2.0 });
    }

    fn touches(&self, touches: &NSSet<UITouch>, phase: Phase) {
        let Some(shared) = self.ivars().shared.borrow().upgrade() else { return };
        for touch in touches {
            let key = Retained::as_ptr(&touch) as usize;
            let id = if phase == Phase::Down {
                let id = self.ivars().next.get().wrapping_add(1);
                self.ivars().next.set(id);
                self.ivars().touches.borrow_mut().insert(key, id);
                id
            } else {
                match self.ivars().touches.borrow().get(&key) {
                    Some(id) => *id,
                    None => continue,
                }
            };
            if matches!(phase, Phase::Up | Phase::Cancel) {
                self.ivars().touches.borrow_mut().remove(&key);
            }
            let at = touch.locationInView(Some(self));
            let press = Press {
                id,
                phase,
                x: at.x,
                y: at.y,
                button: Button::Primary,
                touch: true,
                modifiers: Modifiers::default(),
            };
            shared.press(&press);
        }
    }
}
