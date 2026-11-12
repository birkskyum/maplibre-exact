//! Hello Map on Android: the Contract UI and the TypeScript data module, in the library
//! exact2's Canvas reader loads (`bun exact.mjs android` builds the APK).

#[cfg(target_os = "android")]
mod android {
    #[allow(dead_code)]
    mod baked {
        include!(concat!(env!("OUT_DIR"), "/module.rs"));
    }
    use baked::*;

    type ExactEmbeddedData = exact_js::Placed<exact_js::Module>;
    fn embedded_data() -> ExactEmbeddedData {
        exact_js::Module::new(APP_BYTECODE, APP, GRANTS)
            .with_canvas_surfaces(CANVAS_SURFACES)
            .placed(TYPESCRIPT_PLACEMENT)
    }
    include!(concat!(env!("OUT_DIR"), "/logic.rs"));
    include!(concat!(env!("OUT_DIR"), "/hatches.rs"));

    exact_linux::canvas_jni!(super::AppData, hatches = crate::android::exact_hatches());
}
