use std::path::PathBuf;

/// The libghostty-vt release vendored under `vendor/`.
const VERSION: &str = "1.0.1";

/// Links the prebuilt libghostty-vt static library.
///
/// The archive and headers are checked in under `vendor/` so a build needs no
/// Zig, no CMake and no network.
///
/// They are built from the `vt-1.0.1` tag with the host target, which on Windows
/// is MSVC:
///
/// ```text
/// git clone --depth 1 --branch vt-1.0.1 https://github.com/ghostty-org/ghostty
/// zig build -Demit-lib-vt -Doptimize=ReleaseFast
/// ```
///
/// Ghostty also publishes a Windows zip at
/// `https://release.files.ghostty.org/libghostty-vt/{VERSION}/`, but that one is
/// built for the gnu ABI: its objects use a COMDAT layout `link.exe` rejects
/// with `LNK1143`, so it cannot be linked into an MSVC Rust binary as-is.
///
/// Only Windows is supported, so there is one archive and one import library
/// name.
fn main() {
    let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor");
    let lib = vendor.join("ghostty-vt-static.lib");

    if !lib.exists() {
        panic!(
            "libghostty-vt is missing: {}\n\
             Build it with `zig build -Demit-lib-vt -Doptimize=ReleaseFast` from \
             the vt-{VERSION} tag of https://github.com/ghostty-org/ghostty and \
             copy zig-out/lib/ghostty-vt-static.lib and zig-out/include/ here.",
            lib.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", vendor.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt-static");
    println!("cargo:rerun-if-changed={}", lib.display());
}
