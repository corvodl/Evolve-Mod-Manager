//! Safe command-line functions shared with the desktop application.
use evolve_core::{
    AnimatedGlb, AnimationPolicy, Transform, add_mod_to_profile, animation_references,
    apply_profile_stage, apply_stage_pak, batch_stage_status, convert_glb_to_fbx,
    convert_original_motion, create_mod_profile, delete_mod_profile, enable_profile_mod,
    export_dds_png, export_edited_layer, export_level_scene_glb, export_model_glb,
    export_model_obj, export_rigged_character_fbx, export_rigged_character_glb, export_terrain_obj,
    extract_legacy_pak, extract_signed_pak, import_stage_pak, initialize_stage, inspect_caf_file,
    inspect_dds, inspect_legacy_pak, inspect_signed_pak, list_mod_profiles, load_level_scene,
    plan_mod_profile, read_cdf, read_model, read_terrain, rebuild_legacy_pak,
    rebuild_signed_pak_entry, resolve_cdf, restore_stage, scan_directory, stage_status,
    write_inventory_json,
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
        "  evolve-cli stage init <new-isolated-stage>",
        "  evolve-cli profile list <stage>",
        "  evolve-cli profile status <stage>",
        "  evolve-cli profile recover <stage>",
        "  evolve-cli profile apply <stage> <profile-id> <original-public.der> <mod-public.der>",
        "  evolve-cli profile create <stage> <name>",
        "  evolve-cli profile delete <stage> <profile-id>",
        "  evolve-cli profile add <stage> <profile-id> <replacement.pak> <Game/name.pak> <original-public.der> <mod-public.der> <basename>",
        "  evolve-cli profile enable <stage> <profile-id> <mod-id> <true|false>",
        "  evolve-cli profile plan <stage> <profile-id> <original-public.der> <mod-public.der>",
        "  evolve-cli stage status <stage>",
        "  evolve-cli stage restore <stage>",
        "  evolve-cli stage import <stage> <original.pak> <target-relative.pak> <public.der> <basename>",
        "  evolve-cli stage apply <stage> <replacement.pak> <target-relative.pak> <original.der> <replacement.der> <basename>",
        "  evolve-cli pak inspect <legacy.pak> [pak-inventory.json]",
        "  evolve-cli pak extract <legacy.pak> <new-output-directory>",
        "  evolve-cli pak rebuild <legacy.pak> <overrides-folder> <new-output.pak>",
        "  evolve-cli dds info <texture.dds|texture.dds.0> [dds-info.json]",
        "  evolve-cli dds png <texture.dds|texture.dds.0> <new-output.png>",
        "  evolve-cli cdf inspect <character.cdf> [cdf-info.json]",
        "  evolve-cli material inspect <material.mtl> [material.json]",
        "  evolve-cli material resolve <material.mtl> <asset-library-folder> [material-resolved.json]",
        "  evolve-cli cdf resolve <character.cdf> <asset-library-folder> [cdf-dependencies.json]",
        "  evolve-cli anim caf <motion.caf> [caf-info.json]",
        "  evolve-cli anim catalog <character.chrparams> [catalog.json]",
        "  evolve-cli anim convert <original.caf|original.dba|original.chr> <Cryengine-Converter.exe> <extracted-asset-root> <NEW-output-folder>",
        "  evolve-cli anim clips <animated.glb> [clips.json]",
        "  evolve-cli fbx export <rigged.glb> <Blender.exe> <new.fbx> [--animations]",
        "  evolve-cli signed inspect <original.pak> <public-key.der> <signed-basename> [info.json]",
        "  evolve-cli signed extract <original.pak> <public-key.der> <signed-basename> <new-folder>",
        "  evolve-cli signed rebuild <original.pak> <old-public.der> <new-private.pem> <new-public.der> <basename> <member> <replacement> <new.pak>",
        "  evolve-cli model inspect <model.cgf|model.skin> [model-info.json]",
        "  evolve-cli model obj <model.cgf|model.skin> <new.obj>",
        "  evolve-cli model glb <model.cgf|model.skin> <new.glb>",
        "  evolve-cli rig glb <mesh.skinm> <matching-skeleton.chr> <new.glb>",
        "  evolve-cli rig fbx <mesh.skinm> <matching-skeleton.chr> <Blender.exe> <new.fbx>",
        "  evolve-cli model fbx <model.cgf|model.skin> <Blender.exe> <new.fbx> (no rig yet)",
        "  evolve-cli terrain inspect <Heightmap.dat> [terrain-info.json]",
        "  evolve-cli terrain obj <Heightmap.dat> <new-terrain.obj>",
        "  evolve-cli level inspect <mission.xml|layer.lyr> [scene-info.json]",
        "  evolve-cli level glb <mission.xml|layer.lyr> <asset-folder> <new-scene.glb> [Heightmap.dat]",
        "  evolve-cli level fbx <mission.xml|layer.lyr> <asset-folder> <Blender.exe> <new-scene.fbx> [Heightmap.dat]",
        "  evolve-cli level edit <mission.xml|layer.lyr> <new.xml> <GUID> <x,y,z> [w,x,y,z] [sx,sy,sz]",
        "",
        "Signed Stage 2 PAKs require the matching local PUBLIC RSA key and exact signed basename.",
        "Signed Stage 2 replacement requires a separate local RSA private key; game acceptance is unverified.",
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
    let text = value.to_str().ok_or_else(usage)?;
    let parts = text
        .split(',')
        .map(|v| v.parse::<f32>())
        .collect::<Vec<_>>();
    if parts.len() != N {
        return Err(format!(
            "Expected {N} comma-separated floating point numbers"
        ));
    }
    let mut result = [0.0f32; N];
    for (out, part) in result.iter_mut().zip(parts) {
        *out = part.map_err(|_| "Invalid transform number".to_string())?;
        if !out.is_finite() || out.abs() > 1e8 {
            return Err("Transform outside safe range".into());
        }
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
    if verb == OsStr::new("profile") {
        let action = pop(&mut args)?;
        let stage: PathBuf = pop(&mut args)?.into();
        if action == OsStr::new("status") {
            no_extra(&mut args)?;
            match batch_stage_status(&stage).map_err(|e| e.to_string())? {
                Some(tx) => println!(
                    "Profile {} has {} PAKs requiring recovery ({})",
                    tx.profile_id,
                    tx.entries.len(),
                    tx.id
                ),
                None => println!("No active multi-PAK profile transaction"),
            }
        } else if action == OsStr::new("recover") {
            no_extra(&mut args)?;
            let tx = restore_profile_stage(&stage).map_err(|e| e.to_string())?;
            println!(
                "Recovered {} original PAKs from profile {}",
                tx.entries.len(),
                tx.profile_id
            );
        } else if action == OsStr::new("apply") {
            let id = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let original_key: PathBuf = pop(&mut args)?.into();
            let mod_key: PathBuf = pop(&mut args)?.into();
            no_extra(&mut args)?;
            let tx = apply_profile_stage(&stage, &id, &original_key, &mod_key)
                .map_err(|e| e.to_string())?;
            println!(
                "Staged {} verified signed PAKs; recovery transaction {}",
                tx.entries.len(),
                tx.id
            );
        } else if action == OsStr::new("list") {
            no_extra(&mut args)?;
            let catalog = list_mod_profiles(&stage).map_err(|e| e.to_string())?;
            for profile in catalog.profiles {
                println!(
                    "{} | {} | {} mod(s)",
                    profile.id,
                    profile.name,
                    profile.mods.len()
                );
                for m in profile.mods {
                    println!(
                        "  {} {} -> {} ({})",
                        m.id,
                        m.source,
                        m.target,
                        if m.enabled { "enabled" } else { "disabled" }
                    );
                }
            }
        } else if action == OsStr::new("create") {
            let name = pop(&mut args)?.into_string().map_err(|_| usage())?;
            no_extra(&mut args)?;
            let profile = create_mod_profile(&stage, &name).map_err(|e| e.to_string())?;
            println!(
                "Created offline mod profile {} ({})",
                profile.name, profile.id
            );
        } else if action == OsStr::new("delete") {
            let id = pop(&mut args)?.into_string().map_err(|_| usage())?;
            no_extra(&mut args)?;
            delete_mod_profile(&stage, &id).map_err(|e| e.to_string())?;
            println!("Deleted profile {id}");
        } else if action == OsStr::new("add") {
            let id = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let mod_file: PathBuf = pop(&mut args)?.into();
            let relative = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let original_key: PathBuf = pop(&mut args)?.into();
            let mod_key: PathBuf = pop(&mut args)?.into();
            let basename = pop(&mut args)?.into_string().map_err(|_| usage())?;
            no_extra(&mut args)?;
            let m = add_mod_to_profile(
                &stage,
                &id,
                &mod_file,
                &relative,
                &original_key,
                &mod_key,
                &basename,
            )
            .map_err(|e| e.to_string())?;
            println!("Registered verified mod {} at target {}", m.id, m.target);
        } else if action == OsStr::new("enable") {
            let id = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let mod_id = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let enabled = match pop(&mut args)?.to_str() {
                Some("true") => true,
                Some("false") => false,
                _ => return Err(usage()),
            };
            no_extra(&mut args)?;
            enable_profile_mod(&stage, &id, &mod_id, enabled).map_err(|e| e.to_string())?;
            println!("Updated mod {mod_id} to enabled={enabled}");
        } else if action == OsStr::new("plan") {
            let id = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let original_key: PathBuf = pop(&mut args)?.into();
            let mod_key: PathBuf = pop(&mut args)?.into();
            no_extra(&mut args)?;
            let plan = plan_mod_profile(&stage, &id, &original_key, &mod_key)
                .map_err(|e| e.to_string())?;
            println!(
                "Verified {} enabled signed mods ({} bytes); game unchanged.",
                plan.mods.len(),
                plan.total_bytes
            );
        } else {
            return Err(usage());
        }
        return Ok(());
    }
    if verb == OsStr::new("stage") {
        let action = pop(&mut args)?;
        let root: PathBuf = pop(&mut args)?.into();
        if action == OsStr::new("init") {
            no_extra(&mut args)?;
            initialize_stage(&root).map_err(|e| e.to_string())?;
            println!("Created isolated offline PAK workspace: {}", root.display());
        } else if action == OsStr::new("status") {
            no_extra(&mut args)?;
            match stage_status(&root).map_err(|e| e.to_string())? {
                Some(tx) => println!("Recovery pending for {} ({})", tx.target, tx.id),
                None => println!("No active stage swap journal"),
            }
        } else if action == OsStr::new("restore") {
            no_extra(&mut args)?;
            let tx = restore_stage(&root).map_err(|e| e.to_string())?;
            println!("Restored verified original staged PAK: {}", tx.target);
        } else if action == OsStr::new("import") {
            let original: PathBuf = pop(&mut args)?.into();
            let target = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let public_key: PathBuf = pop(&mut args)?.into();
            let basename = pop(&mut args)?.into_string().map_err(|_| usage())?;
            no_extra(&mut args)?;
            let digest = import_stage_pak(&root, &original, &target, &public_key, &basename)
                .map_err(|e| e.to_string())?;
            println!("Imported verified signed archive into isolated stage (SHA256 {digest})");
        } else if action == OsStr::new("apply") {
            let replacement: PathBuf = pop(&mut args)?.into();
            let target = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let original_key: PathBuf = pop(&mut args)?.into();
            let replacement_key: PathBuf = pop(&mut args)?.into();
            let basename = pop(&mut args)?.into_string().map_err(|_| usage())?;
            no_extra(&mut args)?;
            let tx = apply_stage_pak(
                &root,
                &replacement,
                &target,
                &original_key,
                &replacement_key,
                &basename,
            )
            .map_err(|e| e.to_string())?;
            println!(
                "Applied staged mod {}; recovery journal {}",
                tx.target, tx.id
            );
        } else {
            return Err(usage());
        }
        return Ok(());
    }
    let subcommand = pop(&mut args)?;
    let source: PathBuf = pop(&mut args)?.into();
    if verb == OsStr::new("material") {
        let material = read_material(&source).map_err(|e| e.to_string())?;
        if subcommand == OsStr::new("inspect") {
            let report: Option<PathBuf> = args.next().map(Into::into);
            no_extra(&mut args)?;
            println!("Material contains {} slot(s) with authored texture references.", material.slots.len());
            if let Some(path) = report { write_report(&material, &path)?; }
        } else if subcommand == OsStr::new("resolve") {
            let root: PathBuf = pop(&mut args)?.into();
            let report: Option<PathBuf> = args.next().map(Into::into);
            no_extra(&mut args)?;
            let inventory = scan_directory(&root).map_err(|e| e.to_string())?;
            let resolution = resolve_materials(&inventory, &material);
            println!("Resolved {} exact MTL references with no basename guessing.", resolution.textures.len());
            for item in &resolution.textures {
                println!("  {} / {}: {:?}", item.slot, item.role, item.resolution);
            }
            if let Some(path) = report { write_report(&resolution, &path)?; }
        } else { return Err(usage()); }
        return Ok(());
    }
    if verb == OsStr::new("cdf") && subcommand == OsStr::new("resolve") {
        let asset_root: PathBuf = pop(&mut args)?.into();
        let output: Option<PathBuf> = args.next().map(Into::into);
        no_extra(&mut args)?;
        let character = read_cdf(&source).map_err(|e| e.to_string())?;
        let inventory = scan_directory(&asset_root).map_err(|e| e.to_string())?;
        let result = resolve_cdf(&inventory, &character);
        println!(
            "CDF dependency report: {} attachment(s), skeleton: {:?}",
            result.attachments.len(),
            result.skeleton
        );
        for (binding, resolution) in &result.attachments {
            println!("  {binding}: {resolution:?}");
        }
        if let Some(report) = output {
            write_report(&result, &report)?;
        }
        return Ok(());
    }
    if verb == OsStr::new("signed") && subcommand == OsStr::new("rebuild") {
        let old_public: PathBuf = pop(&mut args)?.into();
        let new_private: PathBuf = pop(&mut args)?.into();
        let new_public: PathBuf = pop(&mut args)?.into();
        let basename = pop(&mut args)?.into_string().map_err(|_| usage())?;
        let member = pop(&mut args)?.into_string().map_err(|_| usage())?;
        let replacement: PathBuf = pop(&mut args)?.into();
        let destination: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        let report = rebuild_signed_pak_entry(
            &source,
            &old_public,
            &new_private,
            &new_public,
            &basename,
            &member,
            &replacement,
            &destination,
        )
        .map_err(|e| e.to_string())?;
        println!(
            "Re-signed and independently verified {} ({} -> {} bytes)",
            report.changed_entry, report.original_bytes, report.replacement_bytes
        );
        println!(
            "Output: {}. Original PAK was not modified.",
            destination.display()
        );
        return Ok(());
    }
    if verb == OsStr::new("level") && subcommand == OsStr::new("glb") {
        let library: PathBuf = pop(&mut args)?.into();
        let destination: PathBuf = pop(&mut args)?.into();
        let terrain: Option<PathBuf> = args.next().map(Into::into);
        no_extra(&mut args)?;
        let report = export_level_scene_glb(&source, &library, terrain.as_deref(), &destination)
            .map_err(|e| e.to_string())?;
        println!(
            "Blender GLB scene: {} model resources, {} placed meshes, {} metadata-only nodes",
            report.exported_mesh_resources,
            report.exported_mesh_instances,
            report.metadata_only_objects
        );
        println!(
            "Output: {} ({} bytes). Geometry-only export.",
            destination.display(),
            report.output_bytes
        );
        return Ok(());
    }
    if verb == OsStr::new("level") && subcommand == OsStr::new("fbx") {
        let library: PathBuf = pop(&mut args)?.into();
        let blender: PathBuf = pop(&mut args)?.into();
        let destination: PathBuf = pop(&mut args)?.into();
        let terrain: Option<PathBuf> = args.next().map(Into::into);
        no_extra(&mut args)?;
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let glb = tmp.path().join("map.glb");
        export_level_scene_glb(&source, &library, terrain.as_deref(), &glb)
            .map_err(|e| e.to_string())?;
        let report = convert_glb_to_fbx(&blender, &glb, &destination, AnimationPolicy::default())
            .map_err(|e| e.to_string())?;
        println!(
            "Blender map scene FBX: {} bytes, source geometry only.",
            report.output_size
        );
        return Ok(());
    }
    if verb == OsStr::new("signed") {
        let key: PathBuf = pop(&mut args)?.into();
        let basename = pop(&mut args)?
            .into_string()
            .map_err(|_| "Signed basename must be UTF-8".to_owned())?;
        let output: Option<PathBuf> = args.next().map(Into::into);
        no_extra(&mut args)?;
        if subcommand == OsStr::new("inspect") {
            let metadata =
                inspect_signed_pak(&source, &key, &basename).map_err(|e| e.to_string())?;
            println!(
                "Verified Stage 2 signed PAK: {} entries ({} expanded bytes)",
                metadata.entries.len(),
                metadata.total_expanded_bytes
            );
            if let Some(path) = output {
                write_report(&metadata, &path)?;
            }
        } else if subcommand == OsStr::new("extract") {
            let target = output.ok_or_else(usage)?;
            let result =
                extract_signed_pak(&source, &key, &basename, &target).map_err(|e| e.to_string())?;
            println!(
                "Verified and extracted {} signed entries into {}",
                result.entries.len(),
                target.display()
            );
        } else {
            return Err(usage());
        }
        return Ok(());
    }
    if verb == OsStr::new("anim") && subcommand == OsStr::new("convert") {
        let converter: PathBuf = pop(&mut args)?.into();
        let asset_root: PathBuf = pop(&mut args)?.into();
        let output: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        let result = convert_original_motion(&converter, &source, &asset_root, &output)
            .map_err(|e| e.to_string())?;
        println!(
            "Verified converter playback for {}: {} animated GLB(s), {} actual clips",
            result.virtual_source,
            result.glb_files.len(),
            result.clips.len()
        );
        for clip in &result.clips {
            println!(
                "  {}  •  {:.3}s  •  {} animated channels",
                clip.name, clip.duration, clip.channels
            );
        }
        return Ok(());
    }
    if verb == OsStr::new("rig") && subcommand == OsStr::new("glb") {
        let skeleton: PathBuf = pop(&mut args)?.into();
        let output: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        let report =
            export_rigged_character_glb(&source, &skeleton, &output).map_err(|e| e.to_string())?;
        println!(
            "Verified source bind poses/skin weights: {} bones, {} weighted vertices, {} subset palettes",
            report.bone_count, report.weighted_vertices, report.subset_count
        );
        println!("Rigged GLB: {}. Embedded animations: no.", output.display());
        return Ok(());
    }
    if verb == OsStr::new("rig") && subcommand == OsStr::new("fbx") {
        let skeleton: PathBuf = pop(&mut args)?.into();
        let blender: PathBuf = pop(&mut args)?.into();
        let output: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        let (rig, fbx) = export_rigged_character_fbx(&source, &skeleton, &blender, &output)
            .map_err(|e| e.to_string())?;
        println!(
            "Rigged FBX source: {} bones, {} active, {} weighted vertices.",
            rig.bone_count, rig.active_bones, rig.weighted_vertices
        );
        println!(
            "Rigged Blender FBX: {} bytes. Embedded animations: no.",
            fbx.output_size
        );
        return Ok(());
    }
    if verb == OsStr::new("model") && subcommand == OsStr::new("fbx") {
        let blender: PathBuf = pop(&mut args)?.into();
        let target: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        let model = read_model(&source).map_err(|e| e.to_string())?;
        let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let intermediate = temp.path().join("source.glb");
        export_model_glb(&model, &intermediate).map_err(|e| e.to_string())?;
        let report =
            convert_glb_to_fbx(&blender, &intermediate, &target, AnimationPolicy::default())
                .map_err(|e| e.to_string())?;
        println!(
            "Exported source geometry as {} bytes of FBX (no rigged skeleton inferred): {}",
            report.output_size,
            target.display()
        );
        return Ok(());
    }
    if verb == OsStr::new("fbx") && subcommand == OsStr::new("export") {
        let blender: PathBuf = pop(&mut args)?.into();
        let dest: PathBuf = pop(&mut args)?.into();
        let embed = match args.next() {
            None => false,
            Some(flag) if flag == OsStr::new("--animations") => true,
            _ => return Err(usage()),
        };
        no_extra(&mut args)?;
        let info = convert_glb_to_fbx(
            &blender,
            &source,
            &dest,
            AnimationPolicy {
                embed_verified_animations: embed,
            },
        )
        .map_err(|e| e.to_string())?;
        println!(
            "Blender FBX created: {} bytes, animations enabled: {}",
            info.output_size, info.animations_included
        );
        return Ok(());
    }
    if verb == OsStr::new("level") && subcommand == OsStr::new("edit") {
        let output: PathBuf = pop(&mut args)?.into();
        let guid = pop(&mut args)?.into_string().map_err(|_| usage())?;
        let pos = parse_vector::<3>(&pop(&mut args)?)?;
        let rot = args
            .next()
            .map(|v| parse_vector::<4>(&v))
            .transpose()?
            .unwrap_or([1., 0., 0., 0.]);
        let scale = args
            .next()
            .map(|v| parse_vector::<3>(&v))
            .transpose()?
            .unwrap_or([1., 1., 1.]);
        no_extra(&mut args)?;
        export_edited_layer(
            &source,
            &output,
            &guid,
            Transform {
                position: pos,
                quaternion: rot,
                scale,
            },
        )
        .map_err(|e| e.to_string())?;
        println!(
            "Saved new placement layer without changing source: {}",
            output.display()
        );
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
    } else if verb == OsStr::new("anim") && subcommand == OsStr::new("caf") {
        let info = inspect_caf_file(&source).map_err(|e| e.to_string())?;
        println!(
            "CAF chunk metadata: {} chunk(s), additive: {}",
            info.chunks, info.additive
        );
        if let Some(path) = output {
            write_report(&info, &path)?;
        }
    } else if verb == OsStr::new("anim") && subcommand == OsStr::new("catalog") {
        let xml = fs::read(&source).map_err(|e| e.to_string())?;
        let refs = animation_references(&xml).map_err(|e| e.to_string())?;
        println!("Character animation references: {}", refs.len());
        if let Some(path) = output {
            write_report(&refs, &path)?;
        }
    } else if verb == OsStr::new("anim") && subcommand == OsStr::new("clips") {
        let timeline = AnimatedGlb::read(&source).map_err(|e| e.to_string())?;
        let clips = timeline.list_clips();
        for clip in &clips {
            println!(
                "{}: {:.3} seconds, {} channels",
                clip.name, clip.duration, clip.channels
            );
        }
        if let Some(path) = output {
            write_report(&clips, &path)?;
        }
    } else if verb == OsStr::new("model") && subcommand == OsStr::new("inspect") {
        let mesh = read_model(&source).map_err(|e| e.to_string())?;
        println!(
            "{} geometry: {} vertices, {} triangles, UV0={}, normals={}",
            mesh.info.format,
            mesh.info.vertex_count,
            mesh.info.triangle_count,
            mesh.info.original_uv0,
            mesh.info.original_normals
        );
        if let Some(path) = output {
            write_report(&mesh.info, &path)?;
        }
    } else if verb == OsStr::new("model") && subcommand == OsStr::new("obj") {
        let mesh = read_model(&source).map_err(|e| e.to_string())?;
        let target = output.ok_or_else(usage)?;
        export_model_obj(&mesh, &target).map_err(|e| e.to_string())?;
        println!(
            "Exported {} original geometry triangles to {}",
            mesh.info.triangle_count,
            target.display()
        );
    } else if verb == OsStr::new("model") && subcommand == OsStr::new("glb") {
        let model = read_model(&source).map_err(|e| e.to_string())?;
        let target = output.ok_or_else(usage)?;
        let bytes = export_model_glb(&model, &target).map_err(|e| e.to_string())?;
        println!(
            "Exported GLB with exact source triangles and UV0 ({} bytes): {}",
            bytes,
            target.display()
        );
    } else if verb == OsStr::new("terrain") && subcommand == OsStr::new("inspect") {
        let mesh = read_terrain(&source, 257).map_err(|e| e.to_string())?;
        println!(
            "Source terrain {}x{}, {} decoded samples, {} visible triangles",
            mesh.info.width,
            mesh.info.height,
            mesh.info.decoded_samples,
            mesh.indices.len() / 3
        );
        if let Some(path) = output {
            write_report(&mesh.info, &path)?;
        }
    } else if verb == OsStr::new("terrain") && subcommand == OsStr::new("obj") {
        let mesh = read_terrain(&source, 257).map_err(|e| e.to_string())?;
        let target = output.ok_or_else(usage)?;
        export_terrain_obj(&mesh, &target).map_err(|e| e.to_string())?;
        println!("Blender-compatible terrain OBJ saved: {}", target.display());
    } else if verb == OsStr::new("level") && subcommand == OsStr::new("inspect") {
        let scene = load_level_scene(&source).map_err(|e| e.to_string())?;
        println!(
            "Level object layer: {} placements, {} diagnostics",
            scene.objects.len(),
            scene.diagnostics.len()
        );
        if let Some(path) = output {
            write_report(&scene, &path)?;
        }
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
