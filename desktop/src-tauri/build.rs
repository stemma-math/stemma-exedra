fn main() {
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
