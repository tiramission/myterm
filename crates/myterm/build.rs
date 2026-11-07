fn main() {
    // The taskbar and the file-manager icon come from resources embedded in the
    // executable, not from `WindowOptions.icon`, which only sets the window's own
    // small icon. Without this the app shows a blank page in the taskbar.
    #[cfg(windows)]
    {
        // rc.exe wants a real ICO, not a PNG: it rejects a PNG with "not in
        // 3.00 format". The ICO is generated from `icon.png` at the sizes Windows
        // asks for (16 through 256), so both files stay in the repository and the
        // ICO is the one the resource compiler sees.
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../icon.ico");
        println!("cargo:rerun-if-changed={}", icon.display());

        // `winresource` wants an absolute path it can hand to rc.exe, and the
        // compiler resolves relative ones against the build directory, so the
        // path is made absolute here rather than left to chance.
        let mut res = winresource::WindowsResource::new();
        res.set_icon(icon.to_str().expect("icon path is not valid UTF-8"));
        res.set("ProductName", "myterm");
        res.set("FileDescription", "A Windows terminal built on GPUI Kit");
        res.set("OriginalFilename", "myterm.exe");
        if let Err(error) = res.compile() {
            // A missing resource compiler should not stop a build that would
            // otherwise work; the app just loses its icon.
            println!("cargo:warning=could not embed the icon: {error}");
        }
    }
}
