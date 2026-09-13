//! The UniFFI bindings generator, in-tree so its version tracks the `uniffi`
//! runtime this crate links. Invoked by `scripts/gen-bindings.sh`:
//!
//! ```sh
//! cargo run -p resvg-swift --bin uniffi-bindgen -- \
//!   generate --library <libresvg_swift.dylib> --language swift --out-dir <dir>
//! ```
fn main() {
    uniffi::uniffi_bindgen_main()
}
