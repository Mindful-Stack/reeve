//! `rust-embed` needs `web/build` to exist at compile time; create it empty so `cargo build` never requires Node.
//! The server then answers 503 "client not built" until `mise run web` fills it.
fn main() {
    std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/build"))
        .expect("create web/build");
    println!("cargo:rerun-if-changed=../../web/build");
}
