// Links the app icon (assets/app.res, made by scripts/icon.py) into the exe.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/app.res");
    println!("cargo:rustc-link-arg-bins={}/../../assets/app.res", env!("CARGO_MANIFEST_DIR"));
}
