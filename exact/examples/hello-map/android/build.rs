fn main() {
    exact_js_bake::build_android(std::path::Path::new("..")).unwrap_or_else(|error| {
        // What the bake found (tsc's and the compiler's diagnostics), as it is: no panic around it.
        eprintln!("{error}");
        std::process::exit(1);
    });
}
