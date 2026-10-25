// Links the app icon (assets/app.res, made by scripts/icon.py) into the DLL: the
// input profile shows icon index 0 in the Windows input method list.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/app.res");
    println!("cargo:rustc-cdylib-link-arg={}/../../assets/app.res", env!("CARGO_MANIFEST_DIR"));
}
