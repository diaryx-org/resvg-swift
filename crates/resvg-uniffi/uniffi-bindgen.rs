//! The UniFFI bindings generator, in-tree so its version tracks the `uniffi`
//! runtime this crate links. Invoked by `scripts/gen-bindings.sh`:
//!
//! ```sh
//! cargo run -p resvg-uniffi --bin uniffi-bindgen -- \
//!   generate <libresvg_uniffi.dylib> --language swift --out-dir <dir>
//! ```
fn main() {
    uniffi::uniffi_bindgen_main()
}
