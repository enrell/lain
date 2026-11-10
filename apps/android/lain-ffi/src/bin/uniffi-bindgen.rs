//! `cargo run --features bindgen --bin uniffi-bindgen -- generate
//! --library <liblain_ffi.so> --language kotlin --out-dir <dir>`
fn main() {
    uniffi::uniffi_bindgen_main()
}
