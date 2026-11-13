//! Takes Hello Map for a walk offscreen, in egui's test harness: the map
//! loads, flies to Zürich, a tap in the middle names Zürich HB, which flying
//! there puts there, and the places fit in view. It saves a frame at each step
//! in `target/tmp/hello-map-walk`. The map fetches its style and tiles, so the
//! walk only runs when asked: `cargo test -p hello-map-egui -- --ignored`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use hello_map_egui::HelloMap;

/// Runs frames for `time` of the clock the map's animations and network
/// requests take.
fn run_for(harness: &mut Harness<'_, HelloMap>, time: Duration) {
    let end = Instant::now() + time;
    while Instant::now() < end {
        harness.step();
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn save(harness: &mut Harness<'_, HelloMap>, folder: &std::path::Path, name: &str) {
    let path = folder.join(format!("{name}.png"));
    harness.render().expect("the frame renders").save(&path).expect("the frame saves");
    println!("{name}: {} · {}", path.display(), harness.state().status);
}

#[test]
#[ignore = "fetches the style and tiles from the network"]
fn walk() {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hello-map-walk");
    std::fs::create_dir_all(&folder).expect("the folder can be made");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 640.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_eframe(|_| HelloMap::new());
    run_for(&mut harness, Duration::from_secs(6));
    save(&mut harness, &folder, "1-loaded");
    harness.get_by_label("Zürich").click();
    run_for(&mut harness, Duration::from_secs(4));
    save(&mut harness, &folder, "2-zurich");
    let middle = harness.state().map.rect().expect("the map is shown").center();
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: middle,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    run_for(&mut harness, Duration::from_secs(2));
    save(&mut harness, &folder, "3-tapped");
    assert_eq!(harness.state().status, "Zürich HB");
    harness.get_by_label("Places").click();
    run_for(&mut harness, Duration::from_secs(3));
    save(&mut harness, &folder, "4-places");
}
