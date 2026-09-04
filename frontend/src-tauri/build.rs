fn main() {
    stage_league_runtime_hash_manifest();
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

fn stage_league_runtime_hash_manifest() {
    println!("cargo:rerun-if-env-changed=MAXGAMESTUDIO_REQUIRE_LEAGUE_RUNTIME_MANIFEST");
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo did not provide CARGO_MANIFEST_DIR"),
    );
    let frontend_dir = manifest_dir
        .parent()
        .expect("Tauri crate should be inside the frontend directory");
    let candidates = [
        manifest_dir
            .join("bundle-resources")
            .join("league-runtime")
            .join("maxgamestudio-runtime-hashes.json"),
        frontend_dir
            .join(".runtime-cache")
            .join("league-runtime")
            .join("maxgamestudio-runtime-hashes.json"),
    ];
    for candidate in &candidates {
        println!("cargo:rerun-if-changed={}", candidate.display());
    }

    let required =
        std::env::var("MAXGAMESTUDIO_REQUIRE_LEAGUE_RUNTIME_MANIFEST").as_deref() == Ok("1");
    let contents = candidates
        .iter()
        .find(|path| path.is_file())
        .map(|path| {
            std::fs::read_to_string(path).unwrap_or_else(|error| {
                panic!(
                    "failed to read League runtime hash manifest {}: {error}",
                    path.display()
                )
            })
        })
        .unwrap_or_else(|| {
            if required {
                panic!("versioned desktop builds require the staged League runtime hash manifest");
            }
            "{}".to_string()
        });
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_str(&contents)
        .unwrap_or_else(|error| panic!("League runtime hash manifest is invalid: {error}"));
    if required && hashes.is_empty() {
        panic!("versioned desktop builds require a non-empty League runtime hash manifest");
    }
    for required_path in ["MaxGameStudioLeague.exe", "resources/app.asar"] {
        if required && !hashes.contains_key(required_path) {
            panic!("League runtime hash manifest is missing required file: {required_path}");
        }
    }
    for (path, digest) in &hashes {
        let normalized = path.replace('\\', "/");
        let safe_path = !normalized.is_empty()
            && !normalized.starts_with('/')
            && !normalized.contains(':')
            && normalized
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..");
        if !safe_path {
            panic!("League runtime hash manifest contains an unsafe path: {path}");
        }
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            panic!("League runtime hash manifest contains an invalid digest: {path}");
        }
    }
    let out_dir = std::path::PathBuf::from(
        std::env::var_os("OUT_DIR").expect("Cargo did not provide OUT_DIR"),
    );
    std::fs::write(out_dir.join("league-runtime-hashes.json"), contents)
        .expect("failed to embed the League runtime hash manifest");
}
