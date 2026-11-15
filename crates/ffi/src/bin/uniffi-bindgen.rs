//! Generates Swift and Kotlin bindings from the compiled library:
//! `cargo run -p beacon-ffi --features bindgen --bin uniffi-bindgen -- generate ...`
//! (see scripts/build-ffi.sh).
fn main() {
    uniffi::uniffi_bindgen_main()
}
