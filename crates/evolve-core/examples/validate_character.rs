//! Read-only acceptance probe. Usage: validate_character <extracted-root>
//! <character.cdf> <base.caf> <layer.caf> <AssetLibrary> [build-label]
//! Game assets and reports belong outside the public repository.
use evolve_core::character_assembly::read_static_cdf_assembly_with_imported;
use evolve_core::{CafLayer, VerifiedCafAssembly, read_native_caf, scan_directory};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !(5..=6).contains(&args.len()) {
        return Err("Expected extracted-root, CDF, base CAF, layer CAF, AssetLibrary and optional build label".into());
    }
    let inventory = scan_directory(Path::new(&args[0]))?;
    let (rest, report) = read_static_cdf_assembly_with_imported(
        Path::new(&args[1]),
        &inventory,
        Some(Path::new(&args[4])),
        args.get(5).map(String::as_str),
    )?;
    println!("{}", serde_json::to_string(&report)?);
    let base = read_native_caf(Path::new(&args[2]))?;
    let overlay = read_native_caf(Path::new(&args[3]))?;
    let skeleton = evolve_core::parse_compiled_skeleton(&std::fs::read(
        report.skeleton_source.as_ref().ok_or("Missing skeleton")?,
    )?)?;
    let known = skeleton
        .controller_ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    println!(
        "{}",
        serde_json::json!({"controllers": base.controllers.len(), "matched":base.controllers.iter().filter(|c| known.contains(&c.bone_crc32)).count()})
    );
    let duration = base.duration_seconds();
    let assembly = VerifiedCafAssembly::from_parts(rest.clone(), &report, base)?;
    for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let seconds = duration * fraction;
        let posed = assembly.mesh_at_layers(
            seconds,
            &[CafLayer {
                motion: &overlay,
                seconds,
                weight: 0.5,
            }],
        )?;
        if posed.indices != rest.indices
            || posed.uv0 != rest.uv0
            || posed.positions.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("Frame violated original topology/UV or finite-position invariants".into());
        }
        let changed = posed
            .positions
            .iter()
            .zip(&rest.positions)
            .filter(|(a, b)| a != b)
            .count();
        println!(
            "{}",
            serde_json::json!({"seconds":seconds,"vertices":posed.positions.len(),"changed_vertices":changed,"parts":assembly.part_count(),"bones":assembly.original_bones()})
        );
    }
    Ok(())
}
