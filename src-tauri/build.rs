fn main() {
    tauri_build::build();

    // MediaRemote is a private framework (System/Library/PrivateFrameworks),
    // which the linker does not search by default. Expose that path so the
    // `#[link(name = "MediaRemote", kind = "framework")]` in media.rs resolves.
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-arg=-F/System/Library/PrivateFrameworks");
    }
}
