fn main() {
    println!("cargo:rerun-if-env-changed=CS2_LIBUNWIND_DLL");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("gnullvm") {
        let source = std::env::var_os("CS2_LIBUNWIND_DLL")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_file())
            .expect("gnullvm builds require CS2_LIBUNWIND_DLL to point to libunwind.dll");
        let out_dir = std::path::PathBuf::from(
            std::env::var_os("OUT_DIR").expect("Cargo did not provide OUT_DIR"),
        );
        let release_dir = out_dir
            .ancestors()
            .nth(3)
            .expect("unable to resolve Cargo profile directory from OUT_DIR");
        let destination = release_dir.join("libunwind.dll");
        // A gnullvm build helper can load the already-staged runtime from the
        // profile directory. Windows then locks that DLL for the lifetime of
        // the helper, so overwriting it here makes every subsequent build fail
        // with ERROR_SHARING_VIOLATION. The release wrapper validates the
        // staged file after the build; this hook only needs to seed it once.
        if !destination.is_file() {
            std::fs::copy(&source, &destination)
                .expect("failed to stage libunwind.dll beside the Tauri executable");
        }
    }

    tauri_build::build();

    // `tauri-build` embeds Common Controls v6 into application binaries, but
    // Cargo's generated Rust test executable is a separate target. Without the
    // same manifest Windows loads comctl32 v5 and fails before the test harness
    // starts because TaskDialogIndirect is unavailable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_RUST_TEST_MANIFEST").is_some()
    {
        embed_resource::compile_for_everything("windows/test-manifest.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed the Windows test manifest");
    }
}
