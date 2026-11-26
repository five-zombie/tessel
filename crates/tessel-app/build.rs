//! Windows-only build step: embed the launcher icon into `tessel.exe`.
//!
//! The `.rc` file is generated into `OUT_DIR` with an absolute icon path so
//! the resource compiler resolves it regardless of its working directory.
//! Non-Windows targets intentionally do nothing here.

fn main() {
    #[cfg(windows)]
    embed_windows_icon();
}

#[cfg(windows)]
fn embed_windows_icon() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    if manifest_dir.is_empty() {
        println!("cargo:warning=CARGO_MANIFEST_DIR unset; skipping Windows icon embed");
        return;
    }
    let icon_path = format!("{manifest_dir}/../../assets/icons/tessel.ico").replace('\\', "/");
    println!("cargo:rerun-if-changed={icon_path}");

    let rc_path = format!(
        "{}/tessel-icon.rc",
        std::env::var("OUT_DIR").unwrap_or_default()
    );
    let rc = format!("1 ICON \"{icon_path}\"\r\n");
    if std::fs::write(&rc_path, rc).is_err() {
        println!("cargo:warning=failed to write Windows .rc file; skipping icon embed");
        return;
    }
    embed_resource::compile(&rc_path, embed_resource::NONE);
}
