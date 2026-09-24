pub fn tempdir() -> tempfile::TempDir {
    let root = std::env::temp_dir()
        .canonicalize()
        .expect("canonical test temp root");
    tempfile::Builder::new()
        .tempdir_in(root)
        .expect("test temp directory")
}
