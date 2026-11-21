use std::env;
use std::path::Path;

fn main() {
    // A test binary gets none of tauri-build's resources. Without a manifest
    // Windows gives it Common Controls 5, which has no TaskDialogIndirect, and
    // stops every test before it runs (STATUS_ENTRYPOINT_NOT_FOUND). So the
    // manifest goes to the linker for every binary, and tauri-build is told
    // not to put a second one into the application.
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("the Tauri build step runs");

    let windows_msvc = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows_msvc {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
