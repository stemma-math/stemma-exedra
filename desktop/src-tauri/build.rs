fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "diagnostics",
            "open_external",
            "login",
            "action",
            "test_sign_in",
            "sign_out",
            "subscribe",
            "unsubscribe",
        ]),
    ))
    .expect("building Stemma");
}
