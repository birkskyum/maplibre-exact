//! Runs Hello Map in a window.

use std::rc::Rc;

use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use hello_map_gpui::HelloMap;

fn main() {
    Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false))).run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(900.0), px(640.0)), cx);
        let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        cx.open_window(options, |_, cx| cx.new(HelloMap::new)).expect("the window opens");
        cx.activate(true);
    });
}
