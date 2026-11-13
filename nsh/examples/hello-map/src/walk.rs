//! The walk `HELLO_MAP_SNAPSHOTS` takes: the window saved at a few moments,
//! the map loaded, flown to Zürich, a tap on Zürich HB, which flying there
//! puts in the middle, and the places fit in view. Then the app quits.
//!
//! The walk taps the map with events of its own, which it hands the map's
//! view directly, and saves the window with `screencapture`, so it needs
//! neither the pointer nor the front of the screen.

use std::path::PathBuf;
use std::time::Duration;

use nsh_core::prelude::*;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message, class};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow};
use objc2_foundation::{NSObjectProtocol, NSPoint};

pub(crate) fn walk(map: ElementRef, status: Signal<String>, folder: PathBuf) {
    std::fs::create_dir_all(&folder).expect("the snapshot folder can be made");
    spawn(async move {
        let snapshot = async |name: &str| {
            let path = folder.join(format!("{name}.png"));
            let saved = run_on_main_thread({
                let path = path.clone();
                move || save(&path)
            })
            .await;
            match saved.map_err(|e| format!("{e:?}")).and_then(|saved| saved) {
                Ok(()) => println!("{name}: {} · {}", path.display(), status.get_untracked()),
                Err(message) => println!("{name}: no snapshot: {message}"),
            }
        };
        timer(Duration::from_secs(6)).await;
        snapshot("1-loaded").await;
        map.command("flyTo", crate::fly_to_zurich());
        timer(Duration::from_secs(4)).await;
        snapshot("2-zurich").await;
        let _ = run_on_main_thread(tap_the_middle).await;
        timer(Duration::from_millis(1500)).await;
        snapshot("3-tapped").await;
        map.command("fitBounds", crate::fit_places());
        timer(Duration::from_millis(2500)).await;
        snapshot("4-places").await;
        std::process::exit(0);
    });
}

fn window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    NSApplication::sharedApplication(mtm).windows().firstObject()
}

/// Saves the app's window, without its shadow, as a PNG file at `path`.
fn save(path: &std::path::Path) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
    let window = window(mtm).ok_or("no window")?;
    let status = std::process::Command::new("screencapture")
        .arg("-x")
        .arg("-o")
        .arg(format!("-l{}", window.windowNumber()))
        .arg(path)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("screencapture: {status}")) }
}

/// The map's view in `view`'s tree.
fn map_view(view: &NSView) -> Option<Retained<NSView>> {
    if view.isKindOfClass(class!(MapLibreNshMapView)) {
        return Some(view.retain());
    }
    view.subviews().iter().find_map(|child| map_view(&child))
}

/// A click in the middle of the map, handed to its view.
fn tap_the_middle() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(window) = window(mtm) else { return };
    let Some(view) = window.contentView().as_deref().and_then(map_view) else { return };
    let bounds = view.bounds();
    let middle = NSPoint::new(bounds.size.width / 2.0, bounds.size.height / 2.0);
    let at = view.convertPoint_toView(middle, None);
    for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            at,
            NSEventModifierFlags::empty(),
            0.0,
            window.windowNumber(),
            None,
            0,
            1,
            1.0,
        );
        let Some(event) = event else { return };
        if kind == NSEventType::LeftMouseDown {
            view.mouseDown(&event);
        } else {
            view.mouseUp(&event);
        }
    }
}
