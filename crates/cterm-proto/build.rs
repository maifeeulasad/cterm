fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile(&["proto/cterm.proto"], &["proto"])?;
    println!("cargo:rerun-if-changed=proto/cterm.proto");
    Ok(())
}
