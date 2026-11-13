//! Exact's canvas input as maplibre-rust-ui's pointers and the gestures'
//! input.

use exact_gpu::{InputEvent, PointerKind as ExactPointer, PointerPhase};
use maplibre_rust_ui::gestures::{Button, Input, Key, Modifiers, PointerKind, WheelDelta};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::{Phase, Pointer};

/// The pointer that `event` reports, if it reports one.
pub(super) fn pointer(event: &InputEvent) -> Option<Pointer> {
    let InputEvent::Pointer { id, phase, x, y, kind, buttons, at_ms, .. } = event else { return None };
    Some(Pointer {
        id: *id,
        phase: match phase {
            PointerPhase::Down => Phase::Down,
            PointerPhase::Move => Phase::Move,
            PointerPhase::Up => Phase::Up,
            PointerPhase::Cancel => Phase::Cancel,
        },
        x: f64::from(*x),
        y: f64::from(*y),
        touch: *kind != ExactPointer::Mouse,
        secondary: buttons & 2 != 0,
        at_ms: *at_ms,
    })
}

/// The recognizer's input for `event`, and when it happened in the host's
/// milliseconds. Exact reports no modifier keys with a pointer or a wheel, so
/// gestures that need one (Shift to box-zoom, Ctrl to rotate) aren't reached.
pub(super) fn translate(event: &InputEvent) -> Option<(Input, f64)> {
    match event {
        InputEvent::Pointer { id, phase, x, y, kind, buttons, at_ms, .. } => {
            let position = ScreenCoordinate::new(f64::from(*x), f64::from(*y));
            let id = *id;
            let input = match phase {
                PointerPhase::Down => Input::PointerDown {
                    id,
                    kind: pointer_kind(*kind),
                    button: button(*buttons),
                    position,
                    modifiers: Modifiers::default(),
                },
                PointerPhase::Move => Input::PointerMove { id, position },
                PointerPhase::Up => Input::PointerUp { id, position },
                PointerPhase::Cancel => Input::PointerCancel { id },
            };
            Some((input, *at_ms))
        }
        InputEvent::Wheel { dy, x, y, at_ms, .. } => Some((
            Input::Wheel {
                position: ScreenCoordinate::new(f64::from(*x), f64::from(*y)),
                delta: WheelDelta::Pixels(f64::from(*dy)),
                modifiers: Modifiers::default(),
            },
            *at_ms,
        )),
        InputEvent::Key { code, key, down: true, at_ms, .. } => {
            map_key(code, key).map(|key| (Input::Key { key, modifiers: Modifiers::default() }, *at_ms))
        }
        InputEvent::Blur { at_ms } => Some((Input::Blur, *at_ms)),
        _ => None,
    }
}

fn pointer_kind(kind: ExactPointer) -> PointerKind {
    match kind {
        ExactPointer::Mouse => PointerKind::Mouse,
        ExactPointer::Touch => PointerKind::Touch,
        ExactPointer::Pen => PointerKind::Pen,
    }
}

/// The button that went down, from the web's `buttons` mask.
fn button(buttons: u32) -> Button {
    if buttons & 2 != 0 {
        Button::Secondary
    } else if buttons & 4 != 0 {
        Button::Middle
    } else {
        Button::Primary
    }
}

fn map_key(code: &str, key: &str) -> Option<Key> {
    match (code, key) {
        ("ArrowLeft", _) => Some(Key::Left),
        ("ArrowRight", _) => Some(Key::Right),
        ("ArrowUp", _) => Some(Key::Up),
        ("ArrowDown", _) => Some(Key::Down),
        ("Escape", _) => Some(Key::Escape),
        (_, "+" | "=") => Some(Key::ZoomIn),
        (_, "-" | "_") => Some(Key::ZoomOut),
        _ => None,
    }
}
