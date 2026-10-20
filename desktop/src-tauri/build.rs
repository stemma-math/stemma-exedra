fn main() {
    // TODO(design): publish the application icons (ICNS, ICO). The bundle
    // configuration uses design's apple-touch-icon.png until they are published.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "login",
            "action",
            "sign_out",
            "subscribe",
            "unsubscribe",
        ]),
    ))
    .expect("building Stemma");
}
