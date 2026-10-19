//! Safe command-line functions shared with the desktop application.
use evolve_core::{
    export_dds_png, extract_legacy_pak, inspect_dds, inspect_legacy_pak, read_cdf,
    rebuild_legacy_pak, scan_directory, write_inventory_json,
    extract_signed_pak, inspect_signed_pak, read_model, export_model_obj,
    load_level_scene, read_terrain, export_terrain_obj, export_edited_layer, Transform,
};
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(message) = run(std::env::args_os().skip(1).collect()) {
        eprintln!("Error: {message}");
        std::process::exit(1);
    }
}

fn usage() -> String {
    [
        "Usage:",
        "  evolve-cli scan <directory> [inventory.json]",
        "  evolve-cli pak inspect <legacy.pak> [pak-inventory.json]",
        "  evolve-cli pak extract <legacy.pak> <new-output-directory>",
        "  evolve-cli pak rebuild <legacy.pak> <overrides-folder> <new-output.pak>",
        "  evolve-cli dds info <texture.dds|texture.dds.0> [dds-info.json]",
        "  evolve-cli dds png <texture.dds|texture.dds.0> <new-output.png>",
        "  evolve-cli cdf inspect <character.cdf> [cdf-info.json]",
        "  evolve-cli signed inspect <original.pak> <public-key.der> <signed-basename> [info.json]",
        "  evolve-cli signed extract <original.pak> <public-key.der> <signed-basename> <new-folder>",
        "  evolve-cli model inspect <model.cgf|model.skin> [model-info.json]",
        "  evolve-cli model obj <model.cgf|model.skin> <new.obj>",
        "  evolve-cli terrain inspect <Heightmap.dat> [terrain-info.json]",
        "  evolve-cli terrain obj <Heightmap.dat> <new-terrain.obj>",
        "  evolve-cli level inspect <mission.xml|layer.lyr> [scene-info.json]",
        "  evolve-cli level edit <mission.xml|layer.lyr> <new.xml> <GUID> <x,y,z> [w,x,y,z] [sx,sy,sz]",
        "",
        "Signed Stage 2 PAKs require the matching local PUBLIC RSA key and exact signed basename.",
        "Native Stage 2 re-signing and mod-injection are NOT available.",
        "Outputs are never silently overwritten.",
    ]
    .join("\n")
}

fn pop(args: &mut std::vec::IntoIter<OsString>) -> Result<OsString, String> {
    args.next().ok_or_else(usage)
}

fn no_extra(args: &mut std::vec::IntoIter<OsString>) -> Result<(), String> {
    if args.next().is_some() {
        Err(usage())
    } else {
        Ok(())
    }
}

fn parse_vector<const N: usize>(value: &OsStr) -> Result<[f32; N], String> {
    let text=value.to_str().ok_or_else(usage)?;
    let parts=text.split(',').map(|v|v.parse::<f32>()).collect::<Vec<_>>();
    if parts.len()!=N {return Err(format!("Expected {N} comma-separated floating point numbers"));}
    let mut result=[0.0f32;N];
    for (out,part) in result.iter_mut().zip(parts){
        *out=part.map_err(|_| "Invalid transform number".to_string())?;
        if !out.is_finite() || out.abs()>1e8 {return Err("Transform outside safe range".into());}
    }
    Ok(result)
}

fn write_report<T: Serialize>(value: &T, path: &Path) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    if let Err(e) = output.write_all(&data).and_then(|_| output.sync_all()) {
        drop(output);
        let _ = fs::remove_file(path);
        return Err(e.to_string());
    }
    println!("Report written: {}", path.display());
    Ok(())
}

fn run(args: Vec<OsString>) -> Result<(), String> {
    let mut args = args.into_iter();
    let verb = pop(&mut args)?;
    if verb == OsStr::new("scan") {
        let source: PathBuf = pop(&mut args)?.into();
        let report: Option<PathBuf> = args.next().map(Into::into);
        no_extra(&mut args)?;
        let inventory = scan_directory(&source).map_err(|e| e.to_string())?;
        println!(
            "Indexed {} files ({} bytes), skipped {} links",
            inventory.entries.len(),
            inventory.total_bytes,
            inventory.skipped_symlinks
        );
        if let Some(report) = report {
            write_inventory_json(&inventory, &report).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let subcommand = pop(&mut args)?;
    let source: PathBuf = pop(&mut args)?.into();
    if verb == OsStr::new("signed") {
        let key: PathBuf = pop(&mut args)?.into();
        let basename = pop(&mut args)?.into_string()
            .map_err(|_| "Signed basename must be UTF-8".to_owned())?;
        let output: Option<PathBuf> = args.next().map(Into::into);
        no_extra(&mut args)?;
        if subcommand == OsStr::new("inspect") {
            let metadata = inspect_signed_pak(&source, &key, &basename).map_err(|e| e.to_string())?;
            println!("Verified Stage 2 signed PAK: {} entries ({} expanded bytes)",
                metadata.entries.len(), metadata.total_expanded_bytes);
            if let Some(path) = output { write_report(&metadata, &path)?; }
        } else if subcommand == OsStr::new("extract") {
            let target = output.ok_or_else(usage)?;
            let result = extract_signed_pak(&source, &key, &basename, &target)
                .map_err(|e| e.to_string())?;
            println!("Verified and extracted {} signed entries into {}",result.entries.len(),target.display());
        } else {
            return Err(usage());
        }
        return Ok(());
    }
    if verb == OsStr::new("level") && subcommand == OsStr::new("edit") {
        let output: PathBuf = pop(&mut args)?.into();
        let guid = pop(&mut args)?.into_string().map_err(|_| usage())?;
        let pos = parse_vector::<3>(&pop(&mut args)?)?;
        let rot = args.next().map(|v| parse_vector::<4>(&v)).transpose()?.unwrap_or([1.,0.,0.,0.]);
        let scale = args.next().map(|v| parse_vector::<3>(&v)).transpose()?.unwrap_or([1.,1.,1.]);
        no_extra(&mut args)?;
        export_edited_layer(&source,&output,&guid,Transform{
            position:pos, quaternion:rot, scale
        }).map_err(|e|e.to_string())?;
        println!("Saved new placement layer without changing source: {}",output.display());
        return Ok(());
    }
    let output: Option<PathBuf> = args.next().map(Into::into);
    let extra: Option<PathBuf> = args.next().map(Into::into);
    no_extra(&mut args)?;
    if (verb != OsStr::new("pak") || subcommand != OsStr::new("rebuild")) && extra.is_some() {
        return Err(usage());
    }
    if verb == OsStr::new("pak") && subcommand == OsStr::new("inspect") {
        let result = inspect_legacy_pak(&source).map_err(|e| e.to_string())?;
        println!(
            "Legacy ZIP PAK: {} files, {} uncompressed bytes",
            result.entries.len(),
            result.total_bytes
        );
        for item in &result.entries {
            println!(
                "  {:>10} {:>10} {}",
                item.size, item.compressed_size, item.path
            );
        }
        if let Some(path) = output {
            write_report(&result, &path)?;
        }
    } else if verb == OsStr::new("pak") && subcommand == OsStr::new("extract") {
        let target = output.ok_or_else(usage)?;
        let extracted = extract_legacy_pak(&source, &target).map_err(|e| e.to_string())?;
        println!(
            "Extracted {} files into {}",
            extracted.entries.len(),
            target.display()
        );
    } else if verb == OsStr::new("pak") && subcommand == OsStr::new("rebuild") {
        let overrides = output.ok_or_else(usage)?;
        let new_pak = extra.ok_or_else(usage)?;
        let report =
            rebuild_legacy_pak(&source, &overrides, &new_pak).map_err(|e| e.to_string())?;
        println!(
            "Rebuilt UNSIGNED historical ZIP PAK with {} replacement(s): {}",
            report.changed_entries,
            new_pak.display()
        );
    } else if verb == OsStr::new("model") && subcommand == OsStr::new("inspect") {
        let mesh=read_model(&source).map_err(|e|e.to_string())?;
        println!("{} geometry: {} vertices, {} triangles, UV0={}, normals={}",
            mesh.info.format,mesh.info.vertex_count,mesh.info.triangle_count,
            mesh.info.original_uv0,mesh.info.original_normals);
        if let Some(path)=output {write_report(&mesh.info,&path)?;}
    } else if verb == OsStr::new("model") && subcommand == OsStr::new("obj") {
        let mesh=read_model(&source).map_err(|e|e.to_string())?;
        let target=output.ok_or_else(usage)?;
        export_model_obj(&mesh,&target).map_err(|e|e.to_string())?;
        println!("Exported {} original geometry triangles to {}",mesh.info.triangle_count,target.display());
    } else if verb == OsStr::new("terrain") && subcommand == OsStr::new("inspect") {
        let mesh=read_terrain(&source,257).map_err(|e|e.to_string())?;
        println!("Source terrain {}x{}, {} decoded samples, {} visible triangles",
            mesh.info.width,mesh.info.height,mesh.info.decoded_samples,mesh.indices.len()/3);
        if let Some(path)=output {write_report(&mesh.info,&path)?;}
    } else if verb == OsStr::new("terrain") && subcommand == OsStr::new("obj") {
        let mesh=read_terrain(&source,257).map_err(|e|e.to_string())?;
        let target=output.ok_or_else(usage)?;
        export_terrain_obj(&mesh,&target).map_err(|e|e.to_string())?;
        println!("Blender-compatible terrain OBJ saved: {}",target.display());
    } else if verb == OsStr::new("level") && subcommand == OsStr::new("inspect") {
        let scene=load_level_scene(&source).map_err(|e|e.to_string())?;
        println!("Level object layer: {} placements, {} diagnostics",
            scene.objects.len(),scene.diagnostics.len());
        if let Some(path)=output {write_report(&scene,&path)?;}
    } else if verb == OsStr::new("dds") && subcommand == OsStr::new("info") {
        let info = inspect_dds(&source).map_err(|e| e.to_string())?;
        println!(
            "{} x {}, {:?}, {} mip(s), {} stream part(s)",
            info.width, info.height, info.format, info.mipmaps, info.streaming_parts
        );
        if let Some(path) = output {
            write_report(&info, &path)?;
        }
    } else if verb == OsStr::new("cdf") && subcommand == OsStr::new("inspect") {
        let cdf = read_cdf(&source).map_err(|e| e.to_string())?;
        println!(
            "CDF: {} | {} valid skin/CHR attachment(s)",
            cdf.skeleton,
            cdf.attachments.len()
        );
        for attachment in &cdf.attachments {
            println!("  {} -> {}", attachment.name, attachment.binding);
        }
        if let Some(path) = output {
            write_report(&cdf, &path)?;
        }
    } else if verb == OsStr::new("dds") && subcommand == OsStr::new("png") {
        let target = output.ok_or_else(usage)?;
        let info = export_dds_png(&source, &target).map_err(|e| e.to_string())?;
        println!(
            "Exported original top mip {} x {} ({:?}) to {}",
            info.width,
            info.height,
            info.format,
            target.display()
        );
    } else {
        return Err(usage());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsupported_usage_without_side_effects() {
        assert!(run(vec![]).is_err());
        assert!(run(vec![OsString::from("pak"), OsString::from("extract")]).is_err());
    }
}
