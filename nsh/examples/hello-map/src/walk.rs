//! The walk the app takes on its own: the map loaded, flown to Zürich, a tap
//! on Zürich HB, which flying there puts in the middle, and the places fit in
//! view. Then the app quits.
//!
//! On macOS the walk taps the map with events of its own, which it hands the
//! map's view directly, and saves the window at each step with
//! `screencapture`, so it needs neither the pointer nor the front of the
//! screen. On iOS it says when it reaches each step, for a screenshot of the
//! simulator, and doesn't tap.

use std::path::PathBuf;
use std::time::Duration;

use nsh_core::prelude::*;

/// Walks, saving the window into `folder` at each step on macOS.
pub(crate) fn walk(map: ElementRef, status: Signal<String>, folder: Option<PathBuf>) {
    if let Some(folder) = &folder {
        std::fs::create_dir_all(folder).expect("the snapshot folder can be made");
    }
    spawn(async move {
        let snapshot = async |name: &str| {
            #[cfg(target_os = "macos")]
            if let Some(folder) = &folder {
                let path = folder.join(format!("{name}.png"));
                let saved = run_on_main_thread({
                    let path = path.clone();
                    move || macos::save(&path)
                })
                .await;
                match saved.map_err(|e| format!("{e:?}")).and_then(|saved| saved) {
                    Ok(()) => println!("{name}: {} · {}", path.display(), status.get_untracked()),
                    Err(message) => println!("{name}: no snapshot: {message}"),
                }
                return;
            }
            let _ = &folder;
            println!("walk: {name} · {}", status.get_untracked());
            timer(Duration::from_secs(2)).await;
        };
        timer(Duration::from_secs(6)).await;
        snapshot("1-loaded").await;
        map.command("flyTo", crate::fly_to_zurich());
        timer(Duration::from_secs(4)).await;
        snapshot("2-zurich").await;
        #[cfg(target_os = "macos")]
        let _ = run_on_main_thread(macos::tap_the_middle).await;
        timer(Duration::from_millis(1500)).await;
        snapshot("3-tapped").await;
        map.command("fitBounds", crate::fit_places());
        timer(Duration::from_millis(2500)).await;
        snapshot("4-places").await;
        std::process::exit(0);
    });
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, Message, class};
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow};
    use objc2_foundation::{NSObjectProtocol, NSPoint};

    fn window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
        NSApplication::sharedApplication(mtm).windows().firstObject()
    }

    /// Saves the app's window, without its shadow, as a PNG file at `path`.
    pub(super) fn save(path: &std::path::Path) -> Result<(), String> {
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
    pub(super) fn tap_the_middle() {
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
}
