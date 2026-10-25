// Compiles proto/MetaRPC.proto with protox (pure Rust, no protoc needed).
// The client is written by hand in src/client.rs so it can keep raw frames;
// only the messages and the server (used by the mock device in tests) are generated.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fds = protox::compile(["MetaRPC.proto"], ["proto"])?;
    tonic_prost_build::configure()
        .build_client(false)
        .build_server(true)
        .compile_fds(fds)?;
    println!("cargo:rerun-if-changed=proto/MetaRPC.proto");
    Ok(())
}
