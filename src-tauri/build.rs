fn main() {
    tauri_build::build();

    // `tauri-build` links its icon/manifest resource only into bin targets.
    // Unit-test binaries also link rfd (through the dialog plugin), which
    // imports TaskDialogIndirect from Common Controls v6. Without a manifest
    // the loader binds comctl32 v5 and the test binary fails with
    // STATUS_ENTRYPOINT_NOT_FOUND. A `.drectve` manifest dependency makes
    // link.exe emit the manifest for test builds only.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
        let directive = concat!(
            "/MANIFESTDEPENDENCY:\"type='win32' ",
            "name='Microsoft.Windows.Common-Controls' version='6.0.0.0' ",
            "processorArchitecture='*' publicKeyToken='6595b64144ccf1df' ",
            "language='*'\" "
        );
        std::fs::write(
            std::path::Path::new(&out_dir).join("test-manifest.drectve"),
            directive,
        )
        .expect("write test manifest directive");
    }
}
