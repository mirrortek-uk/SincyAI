//! WP-02: compile proto/*.proto with protox (pure-Rust protoc replacement,
//! so CI/dev machines need no protoc binary) and generate prost bindings.

use std::path::Path;

fn main() {
    let proto_dir = Path::new("../../proto");
    println!("cargo:rerun-if-changed={}", proto_dir.display());
    let files = [
        "envelope.proto",
        "device.proto",
        "publishing.proto",
        "transfer.proto",
        "adapter.proto",
    ]
    .iter()
    .map(|f| proto_dir.join(f))
    .collect::<Vec<_>>();
    let includes = [proto_dir.to_path_buf()];

    let fds = protox::compile(&files, &includes).expect("protox compile failed");
    prost_build::Config::new()
        .compile_fds(fds)
        .expect("prost compile_fds failed");
}
