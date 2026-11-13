//! A short walk offscreen, in GPUI's headless context with its Metal
//! renderer: the map loads, flies to Zürich, a tap in the middle names Zürich
//! HB, and the places fit in view. Each step is saved as a PNG file in
//! `target/tmp/hello-map-walk`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, HeadlessAppContext, PlatformHeadlessRenderer, WindowHandle, px, size};
use gpui_apple::AppleTextSystem;
use gpui_macos::metal_renderer::MetalHeadlessRenderer;
use hello_map_gpui::HelloMap;
use maplibre_gpui::{Button, Modifiers, Phase, Press};

/// Draws the window's frames for `duration`, as a display would.
fn run(cx: &mut HeadlessAppContext, window: WindowHandle<HelloMap>, duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx)).expect("the window draws");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn save(cx: &mut HeadlessAppContext, window: WindowHandle<HelloMap>, name: &str) {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hello-map-walk");
    std::fs::create_dir_all(&folder).expect("the folder can be made");
    let image = cx.capture_screenshot(window.into()).expect("the window renders to an image");
    image.save(folder.join(format!("{name}.png"))).expect("the image saves");
    let status = cx.update(|cx| window.read(cx).map(|app| app.status.clone())).expect("the app is there");
    println!("{name}: {status}");
}

#[test]
#[ignore = "loads its tiles from the network"]
fn walk() {
    // The Mac platform's own text system, which the platform makes only on the main thread.
    let text_system = Arc::new(AppleTextSystem::new());
    let renderer = || Ok(Some(Box::new(MetalHeadlessRenderer::new()?) as Box<dyn PlatformHeadlessRenderer>));
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(()), renderer);
    let window = cx.open_window(size(px(900.0), px(640.0)), |_, cx| cx.new(HelloMap::new)).expect("the window opens");

    run(&mut cx, window, Duration::from_secs(8));
    save(&mut cx, window, "1-loaded");

    cx.update(|cx| window.update(cx, |app, _, cx| app.fly_to_zurich(cx))).expect("the app flies");
    run(&mut cx, window, Duration::from_secs(4));
    save(&mut cx, window, "2-zurich");

    cx.update(|cx| {
        window.update(cx, |app, _, cx| {
            app.map.update(cx, |map, cx| {
                let viewport = map.viewport().expect("the map has a size");
                let (x, y) = (f64::from(viewport.size.width) / 2.0, f64::from(viewport.size.height) / 2.0);
                for phase in [Phase::Down, Phase::Up] {
                    let press = Press {
                        id: 0,
                        phase,
                        x,
                        y,
                        button: Button::Primary,
                        touch: false,
                        modifiers: Modifiers::default(),
                    };
                    map.pointer(&press, cx);
                }
            });
        })
    })
    .expect("the map takes the tap");
    run(&mut cx, window, Duration::from_millis(1500));
    save(&mut cx, window, "3-tapped");
    let status = cx.update(|cx| window.read(cx).map(|app| app.status.clone())).expect("the app is there");
    assert_eq!(status.as_ref(), "Zürich HB");

    cx.update(|cx| window.update(cx, |app, _, cx| app.fit_places(cx))).expect("the app fits the places");
    run(&mut cx, window, Duration::from_secs(3));
    save(&mut cx, window, "4-places");
}
