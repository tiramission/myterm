use std::path::PathBuf;

/// Links the prebuilt libghostty-vt static library.
///
/// The archive is checked in under `vendor/` because the library is built
/// with Zig, and Ghostty publishes no Windows package. Only Windows is
/// supported, so there is one archive and one import library name.
fn main() {
    let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor");
    let lib = vendor.join("ghostty-vt-static.lib");

    if !lib.exists() {
        panic!(
            "libghostty-vt is missing: {}\n\
             Rebuild it with `zig build -Demit-lib-vt -Doptimize=ReleaseFast` \
             from the 1.0.0 source and copy zig-out/lib/ghostty-vt-static.lib here.",
            lib.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", vendor.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt-static");
    println!("cargo:rerun-if-changed={}", lib.display());
}
