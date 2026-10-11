fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure().compile_protos(&["proto/cterm.proto"], &["proto"])?;
    println!("cargo:rerun-if-changed=proto/cterm.proto");
    Ok(())
}
