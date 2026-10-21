//! Export acceptance probe. Outputs must stay outside original assets and Git.
//! Usage: validate_unified_export <root> <cdf> <blender.exe> <new-output.fbx> [clip.caf]
use evolve_core::{export_unified_cdf_fbx, scan_directory};
use std::path::Path;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !(4..=5).contains(&args.len()) {
        return Err("Expected root, CDF, Blender, new FBX output and optional CAF".into());
    }
    let inventory = scan_directory(Path::new(&args[0]))?;
    let report = export_unified_cdf_fbx(
        Path::new(&args[1]),
        &inventory,
        Path::new(&args[2]),
        Path::new(&args[3]),
        args.get(4).map(Path::new),
    )?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
