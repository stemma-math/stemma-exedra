//! Rebuild the hub when its embedded web app changes.

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let build = root.join("web/build");
    // Watching a path that does not exist would rebuild the hub every time,
    // so make sure the folder is there; in a read-only tree, do without.
    let _ = std::fs::create_dir_all(&build);
    println!("cargo::rerun-if-changed={}", build.display());
}
