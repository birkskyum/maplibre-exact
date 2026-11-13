//! How `whisker run` builds and launches Hello Map.

fn main() {
    whisker_cng::run(|app| {
        app.name("HelloMap").bundle_id("org.maplibre.rustui.hellomap").version("0.0.1").build_number(1);
        app.android(|android| {
            android
                .package("org.maplibre.rustui.hellomap")
                .application_id("org.maplibre.rustui.hellomap")
                .launcher_activity(".MainActivity")
                .min_sdk(24)
                .target_sdk(34);
        });
        app.ios(|ios| {
            ios.bundle_id("org.maplibre.rustui.hellomap").scheme("HelloMap").deployment_target("15.0");
        });
    });
}
