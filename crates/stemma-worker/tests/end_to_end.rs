//! Scratch files are ephemeral; credentials and launch files stay outside the workspace.
#[test]
fn scratch_is_removed_after_cancellation_or_completion() {
    let root = tempfile::tempdir().unwrap();
    let scratch = stemma_worker::workspace::Scratch::create(root.path(), 12).unwrap();
    let path = scratch.0.clone();
    std::fs::write(path.join("notes"), "ephemeral").unwrap();
    assert!(path.exists());
    drop(scratch);
    assert!(!path.exists());
}
