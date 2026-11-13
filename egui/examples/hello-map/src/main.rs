//! Runs Hello Map in a window.

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title("Hello Map").with_inner_size([900.0, 640.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("Hello Map", options, Box::new(|_| Ok(Box::new(hello_map_egui::HelloMap::new()))))
}
