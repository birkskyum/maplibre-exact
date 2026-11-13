//! Compiles the app's UI, with maplibre-slint's library as `@maplibre`.

fn main() {
    let ui = std::env::var("DEP_MAPLIBRE_SLINT_UI").expect("maplibre-slint's build gives the path of its UI");
    let config = slint_build::CompilerConfiguration::new()
        .with_library_paths(std::collections::HashMap::from([("maplibre".to_owned(), ui.into())]));
    slint_build::compile_with_config("ui/app.slint", config).expect("ui/app.slint compiles");
}
