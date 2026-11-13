//! Runs Hello Map on the machine's native backend (`cargo run`), with the map
//! registered.

#[cfg(target_os = "macos")]
fn main() -> impl std::process::Termination {
    use nsh_platform_macos as platform;
    platform::set_components(maplibre_nsh::register);
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<hello_map_nsh::App>()),
        platform::AppOptions {
            title: "Hello Map".to_owned(),
            size: (900, 640),
            // The walk's window shows without taking the screen's front.
            background: std::env::var_os("HELLO_MAP_SNAPSHOTS").is_some(),
            ..Default::default()
        },
    )
}

#[cfg(not(target_os = "macos"))]
compile_error!("Hello Map runs on macOS so far.");
