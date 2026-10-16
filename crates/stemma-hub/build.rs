//! Rebuilds the hub when the web app it embeds is rebuilt.

fn main() {
    let build = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/build");
    // Watching a path that does not exist would rebuild the hub every time,
    // so make sure the folder is there; in a read-only tree, do without.
    let _ = std::fs::create_dir_all(&build);
    println!("cargo::rerun-if-changed={}", build.display());
}
