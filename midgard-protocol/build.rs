fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::compile_protos("proto/midgard/operator/v1/operator.proto")?;
    println!("cargo:rerun-if-changed=proto/midgard/operator/v1/operator.proto");
    Ok(())
}
