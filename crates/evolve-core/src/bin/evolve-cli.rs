//! Safe command-line functions shared with the desktop application.
use evolve_core::signed_pak::{extract_signed_member, rebuild_signed_pak_with_additions};
use evolve_core::{
    AnimatedGlb, AnimationPolicy, Transform, add_mod_to_profile, animation_references,
    apply_profile_stage, apply_stage_pak, apply_stage_pak_with_additions, batch_stage_status,
    convert_glb_to_fbx, convert_original_motion, create_mod_profile, delete_mod_profile,
    enable_profile_mod, export_dds_png, export_edited_layer, export_level_scene_glb,
    export_model_glb, export_model_obj, export_rigged_character_fbx, export_rigged_character_glb,
    export_terrain_obj, extract_legacy_pak, extract_signed_pak, import_stage_pak, initialize_stage,
    inspect_caf_file, inspect_client_hook_pair, inspect_dds, inspect_legacy_pak,
    inspect_pak_hook_pair, inspect_signed_pak, list_mod_profiles, load_level_scene,
    plan_mod_profile, prepare_client_hook, read_cdf, read_material, read_model, read_terrain,
    rebuild_legacy_pak, rebuild_signed_pak_entry, resign_signed_pak, resolve_cdf,
    resolve_materials, restore_profile_stage, restore_stage, scan_directory, stage_status,
    write_inventory_json,
};
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const OVERLAY_MAX_MEMBERS: usize = 64;

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
        "  evolve-cli cryxml decode <binary-xml> <new.xml>",
        "  evolve-cli cryxml encode <edited.xml> <new-binary-xml>",
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
        "  evolve-cli stage apply-add <stage> <replacement.pak> <target-relative.pak> <original.der> <replacement.der> <basename> (isolated stage only)",
        "  evolve-cli pak inspect <legacy.pak> [pak-inventory.json]",
        "  evolve-cli pak extract <legacy.pak> <new-output-directory>",
        "  evolve-cli pak rebuild <legacy.pak> <overrides-folder> <new-output.pak>",
        "  evolve-cli dds info <texture.dds|texture.dds.0> [dds-info.json]",
        "  evolve-cli dds png <texture.dds|texture.dds.0> <new-output.png>",
        "  evolve-cli cdf inspect <character.cdf> [cdf-info.json]",
        "  evolve-cli material inspect <material.mtl> [material.json]",
        "  evolve-cli material effects <Libs/MaterialEffects-folder> <source-surface> <target-surface> [report.json] (read-only)",
        "  evolve-cli material resolve <material.mtl> <asset-library-folder> [material-resolved.json]",
        "  evolve-cli cdf resolve <character.cdf> <asset-library-folder> [cdf-dependencies.json]",
        "  evolve-cli anim caf <motion.caf> [caf-info.json]",
        "  evolve-cli anim catalog <character.chrparams> [catalog.json]",
        "  evolve-cli anim convert <original.caf|original.dba|original.chr> <Cryengine-Converter.exe> <extracted-asset-root> <NEW-output-folder>",
        "  evolve-cli anim clips <animated.glb> [clips.json]",
        "  evolve-cli fbx export <rigged.glb> <Blender.exe> <new.fbx> [--animations]",
        "  evolve-cli signed inspect <original.pak> <public-key.der> <signed-basename> [info.json]",
        "  evolve-cli signed extract <original.pak> <public-key.der> <signed-basename> <new-folder>",
        "  evolve-cli signed extract-member <original.pak> <public-key.der> <signed-basename> <exact-member-path> <new-file>",
        "  evolve-cli signed rebuild <original.pak> <old-public.der> <new-private.pem> <new-public.der> <basename> <member> <replacement> <new.pak>",
        "  evolve-cli signed overlay <original.pak> <old-public.der> <new-private.pem> <new-public.der> <basename> <source-folder> <new.pak>",
        "  evolve-cli signed resign <original.pak> <old-public.der> <new-private.pem> <new-public.der> <basename> <new.pak>",
        "  evolve-cli hook prepare <original-inject.dll> <old-public.der> <new-public.der> <new-staging/inject.dll>",
        "  evolve-cli hook verify <original-inject.dll> <custom-inject.dll> <old-public.der> <new-public.der>",
        "  evolve-cli hook pak-preflight <original.pak> <custom.pak> <original-inject.dll> <custom-inject.dll> <old-public.der> <new-public.der> <basename>",
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
        "Overlay rebuild accepts 1-64 regular files; symlinks/reparse points are skipped and duplicate member names are refused.",
        "Hook preparation only writes a NEW copy; it does not patch or run the installed client.",
        "PAK/hook preflight does not verify Evolve mounted the PAK; real offline Stage 2 launch testing remains necessary.",
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

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn overlay_member_name(relative: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            return Err("Overlay file paths must stay inside the source folder".into());
        };
        parts.push(
            part.to_str()
                .ok_or_else(|| "Overlay file names must be valid UTF-8".to_owned())?,
        );
    }
    if parts.is_empty() {
        return Err("Overlay member path is empty".into());
    }
    let name = parts.join("/");
    if evolve_core::archive::safe_member_name(&name).map_err(|e| e.to_string())? != name {
        return Err(format!("Unsafe overlay member path: {name}"));
    }
    Ok(name)
}

fn collect_overlay_dir(
    root: &Path,
    folder: &Path,
    files: &mut Vec<(String, PathBuf)>,
    skipped_links: &mut usize,
) -> Result<(), String> {
    let mut entries = fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if is_link_or_reparse(&metadata) {
            *skipped_links += 1;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "Overlay path escaped the source folder".to_owned())?;
        let member = overlay_member_name(relative)?;
        if metadata.file_type().is_dir() {
            collect_overlay_dir(root, &path, files, skipped_links)?;
        } else if metadata.file_type().is_file() {
            if files.len() >= OVERLAY_MAX_MEMBERS {
                return Err(format!(
                    "Overlay source must contain at most {OVERLAY_MAX_MEMBERS} files"
                ));
            }
            files.push((member, path));
        } else {
            return Err(format!(
                "Overlay source contains a non-regular file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn collect_overlay_files(root: &Path) -> Result<(Vec<(String, PathBuf)>, usize), String> {
    let metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_dir() || is_link_or_reparse(&metadata) {
        return Err(
            "Overlay source must be a regular directory, not a link or reparse point".into(),
        );
    }
    let mut files = Vec::new();
    let mut skipped_links = 0;
    collect_overlay_dir(root, root, &mut files, &mut skipped_links)?;
    if files.is_empty() {
        return Err("Overlay source folder contains no files".into());
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen = HashSet::new();
    for (member, _) in &files {
        if !seen.insert(member.to_ascii_lowercase()) {
            return Err(format!("Duplicate overlay member path: {member}"));
        }
    }
    Ok((files, skipped_links))
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

fn inspect_surface_pair(
    root: &Path,
    source: &str,
    target: &str,
) -> Result<serde_json::Value, String> {
    use evolve_core::material_effects::{
        MatrixLookup, read_effect_matrix, read_fx_library, read_surface_types,
    };
    let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_dir() || is_link_or_reparse(&metadata) {
        return Err("Select a regular, same-build MaterialEffects folder".into());
    }
    let surfaces =
        read_surface_types(&root.join("SurfaceTypes.xml")).map_err(|error| error.to_string())?;
    let matrix =
        read_effect_matrix(&root.join("MaterialEffects.xml")).map_err(|error| error.to_string())?;
    let left = surfaces.get(source);
    let right = surfaces.get(target);
    let mut status = "not_defined";
    let mut authored = None;
    let mut resolved = None;
    if left.is_some() && right.is_some() {
        match matrix.lookup_pair(source, target) {
            MatrixLookup::NotDefined { .. } => {}
            MatrixLookup::Missing { .. } => status = "missing_cell",
            MatrixLookup::Blank { .. } => status = "explicitly_blank",
            MatrixLookup::Invalid { cell, .. } => {
                status = "invalid_cell";
                authored = Some(cell.raw.clone());
            }
            MatrixLookup::Effect { reference, .. } => {
                status = "effect_missing_library";
                authored = Some(reference.raw.clone());
                let path = root
                    .join("FXLibs")
                    .join(format!("{}.xml", reference.library));
                if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
                    let library = read_fx_library(&path).map_err(|error| error.to_string())?;
                    if let Some(effect) = library.effect(&reference.effect) {
                        status = "effect_found";
                        resolved = Some(effect.clone());
                    } else {
                        status = "effect_missing_definition";
                    }
                }
            }
        }
    }
    Ok(serde_json::json!({
        "source": left,
        "target": right,
        "status": status,
        "authored_effect": authored,
        "effect": resolved,
        "surface_count": surfaces.surfaces.len(),
        "matrix_rows": matrix.rows.len(),
        "matrix_columns": matrix.columns.len(),
        "surface_without_matrix_row_or_column": matrix.surfaces_without_row_or_column(&surfaces),
        "note": "Read-only same-folder inspection; no physical simulation, sound or particle execution"
    }))
}

fn run(args: Vec<OsString>) -> Result<(), String> {
    let mut args = args.into_iter();
    let verb = pop(&mut args)?;
    if verb == OsStr::new("cryxml") {
        let action = pop(&mut args)?;
        let source: PathBuf = pop(&mut args)?.into();
        let target: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        const LIMIT: usize = 48 * 1024 * 1024;
        let metadata = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > LIMIT as u64 {
            return Err("Select a bounded regular XML source file".into());
        }
        let bytes = fs::read(&source).map_err(|e| e.to_string())?;
        let result = if action == OsStr::new("encode") {
            evolve_core::cryxml::encode_xml(&bytes, LIMIT).map_err(|e| e.to_string())?
        } else if action == OsStr::new("decode") && bytes.starts_with(b"CryXmlB\0") {
            evolve_core::cryxml::xml_bytes(&bytes, LIMIT)
                .map_err(|e| e.to_string())?
                .into_owned()
        } else {
            return Err("Use encode with text XML or decode with CryXmlB".into());
        };
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_dir()) {
            return Err("Output directory must be an existing regular directory".into());
        }
        let mut pending = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        pending
            .write_all(&result)
            .and_then(|_| pending.as_file_mut().sync_all())
            .map_err(|e| e.to_string())?;
        pending
            .persist_noclobber(&target)
            .map_err(|e| e.error.to_string())?;
        println!(
            "Wrote {} verified XML bytes to {}",
            result.len(),
            target.display()
        );
        return Ok(());
    }
    if verb == OsStr::new("hook") {
        let action = pop(&mut args)?;
        let original_hook: PathBuf = pop(&mut args)?.into();
        if action == OsStr::new("prepare") {
            let original_key: PathBuf = pop(&mut args)?.into();
            let custom_key: PathBuf = pop(&mut args)?.into();
            let output: PathBuf = pop(&mut args)?.into();
            no_extra(&mut args)?;
            let report = prepare_client_hook(&original_hook, &original_key, &custom_key, &output)
                .map_err(|e| e.to_string())?;
            println!(
                "Prepared new hook (only the RSA public-key bytes changed): {}",
                output.display()
            );
            println!(
                "Original SHA256: {}; custom SHA256: {}",
                report.source_sha256, report.custom_sha256
            );
        } else if action == OsStr::new("verify") {
            let custom_hook: PathBuf = pop(&mut args)?.into();
            let original_key: PathBuf = pop(&mut args)?.into();
            let custom_key: PathBuf = pop(&mut args)?.into();
            no_extra(&mut args)?;
            let report =
                inspect_client_hook_pair(&original_hook, &custom_hook, &original_key, &custom_key)
                    .map_err(|e| e.to_string())?;
            println!("PASS: original and custom hook differ only in the exact RSA key region.");
            println!("Hook SHA256: {}", report.custom_sha256);
        } else if action == OsStr::new("pak-preflight") {
            let custom_pak: PathBuf = pop(&mut args)?.into();
            let original_shim: PathBuf = pop(&mut args)?.into();
            let custom_shim: PathBuf = pop(&mut args)?.into();
            let original_key: PathBuf = pop(&mut args)?.into();
            let custom_key: PathBuf = pop(&mut args)?.into();
            let basename = pop(&mut args)?.into_string().map_err(|_| usage())?;
            no_extra(&mut args)?;
            let report = inspect_pak_hook_pair(
                &original_hook,
                &custom_pak,
                &original_shim,
                &custom_shim,
                &original_key,
                &custom_key,
                &basename,
            )
            .map_err(|e| e.to_string())?;
            println!(
                "PASS: {} signed entries authenticate and both hooks match their respective public keys.",
                report.signed_replacement_entries
            );
            println!("Runtime PAK mount still UNVERIFIED. No installed game file was changed.");
        } else {
            return Err(usage());
        }
        return Ok(());
    }
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
            for change in &plan.member_changes {
                println!(
                    "  {}: +{} added, {} changed, -{} removed, {} case-renamed members{}",
                    change.target,
                    change.added,
                    change.changed,
                    change.removed,
                    change.case_renamed,
                    if change.removed > 0 || change.case_renamed > 0 {
                        " (compatibility warning)"
                    } else {
                        ""
                    }
                );
            }
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
        } else if action == OsStr::new("apply") || action == OsStr::new("apply-add") {
            let replacement: PathBuf = pop(&mut args)?.into();
            let target = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let original_key: PathBuf = pop(&mut args)?.into();
            let replacement_key: PathBuf = pop(&mut args)?.into();
            let basename = pop(&mut args)?.into_string().map_err(|_| usage())?;
            no_extra(&mut args)?;
            let apply = if action == OsStr::new("apply-add") {
                apply_stage_pak_with_additions
            } else {
                apply_stage_pak
            };
            let tx = apply(
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
        if subcommand == OsStr::new("effects") {
            let from = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let to = pop(&mut args)?.into_string().map_err(|_| usage())?;
            let report: Option<PathBuf> = args.next().map(Into::into);
            no_extra(&mut args)?;
            let result = inspect_surface_pair(&source, &from, &to)?;
            println!("Surface interaction: {}", result["status"]);
            if let Some(output) = report {
                write_report(&result, &output)?;
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
                );
            }
            return Ok(());
        }
        let material = read_material(&source).map_err(|e| e.to_string())?;
        if subcommand == OsStr::new("inspect") {
            let report: Option<PathBuf> = args.next().map(Into::into);
            no_extra(&mut args)?;
            println!(
                "Material contains {} slot(s) with authored texture references.",
                material.slots.len()
            );
            if let Some(path) = report {
                write_report(&material, &path)?;
            }
        } else if subcommand == OsStr::new("resolve") {
            let root: PathBuf = pop(&mut args)?.into();
            let report: Option<PathBuf> = args.next().map(Into::into);
            no_extra(&mut args)?;
            let inventory = scan_directory(&root).map_err(|e| e.to_string())?;
            let resolution = resolve_materials(&inventory, &material);
            println!(
                "Resolved {} exact MTL references with no basename guessing.",
                resolution.textures.len()
            );
            for item in &resolution.textures {
                println!("  {} / {}: {:?}", item.slot, item.role, item.resolution);
            }
            if let Some(path) = report {
                write_report(&resolution, &path)?;
            }
        } else {
            return Err(usage());
        }
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
    if verb == OsStr::new("signed") && subcommand == OsStr::new("resign") {
        let old_public: PathBuf = pop(&mut args)?.into();
        let new_private: PathBuf = pop(&mut args)?.into();
        let new_public: PathBuf = pop(&mut args)?.into();
        let basename = pop(&mut args)?.into_string().map_err(|_| usage())?;
        let destination: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        let report = resign_signed_pak(
            &source,
            &old_public,
            &new_private,
            &new_public,
            &basename,
            &destination,
        )
        .map_err(|e| e.to_string())?;
        println!(
            "Re-signed {} original entries without modifying any PAK payloads or ZIP offsets.",
            report.entries.len()
        );
        println!(
            "Authenticated with the custom public key: {}. The installed client was not changed.",
            destination.display()
        );
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
    if verb == OsStr::new("signed") && subcommand == OsStr::new("overlay") {
        let old_public: PathBuf = pop(&mut args)?.into();
        let new_private: PathBuf = pop(&mut args)?.into();
        let new_public: PathBuf = pop(&mut args)?.into();
        let basename = pop(&mut args)?
            .into_string()
            .map_err(|_| "Signed basename must be UTF-8".to_owned())?;
        let folder: PathBuf = pop(&mut args)?.into();
        let destination: PathBuf = pop(&mut args)?.into();
        no_extra(&mut args)?;
        let (changes, skipped_links) = collect_overlay_files(&folder)?;
        if skipped_links != 0 {
            println!("Skipped {skipped_links} symlink/reparse point(s) in the overlay source.");
        }
        let reports = rebuild_signed_pak_with_additions(
            &source,
            &old_public,
            &new_private,
            &new_public,
            &basename,
            &changes,
            &destination,
        )
        .map_err(|e| e.to_string())?;
        println!(
            "Rebuilt and independently verified {} overlay member(s) with the new public key.",
            reports.len()
        );
        for report in &reports {
            println!(
                "  {} ({} -> {} bytes)",
                report.changed_entry, report.original_bytes, report.replacement_bytes
            );
        }
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
        if subcommand == OsStr::new("extract-member") {
            let member = pop(&mut args)?
                .into_string()
                .map_err(|_| "Signed member must be UTF-8".to_owned())?;
            let target: PathBuf = pop(&mut args)?.into();
            no_extra(&mut args)?;
            let entry = extract_signed_member(&source, &key, &basename, &member, &target)
                .map_err(|e| e.to_string())?;
            println!(
                "Verified and extracted {} ({} bytes) to {}",
                entry.path,
                entry.size,
                target.display()
            );
            return Ok(());
        }
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
    fn surface_effect_inspection_stays_inside_selected_folder() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("FXLibs")).unwrap();
        fs::write(root.path().join("SurfaceTypes.xml"),
            br#"<SurfaceTypes><SurfaceType name="mat_metal"/><SurfaceType name="mat_wood"/></SurfaceTypes>"#).unwrap();
        fs::write(root.path().join("MaterialEffects.xml"), br#"<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet><Table><Row><Cell/><Cell><Data>mat_wood</Data></Cell></Row><Row><Cell><Data>mat_metal</Data></Cell><Cell><Data>collisions:hit</Data></Cell></Row></Table></Worksheet></Workbook>"#).unwrap();
        fs::write(root.path().join("FXLibs/collisions.xml"), br#"<FXLib><Effect name="hit"><Sound><Name>sound/reference</Name></Sound></Effect></FXLib>"#).unwrap();
        let report = inspect_surface_pair(root.path(), "mat_metal", "mat_wood").unwrap();
        assert_eq!(report["status"], "effect_found");
        assert_eq!(
            report["effect"]["resources"][0]["reference"],
            "sound/reference"
        );
        assert_eq!(
            inspect_surface_pair(root.path(), "mat_unknown", "mat_wood").unwrap()["status"],
            "not_defined"
        );
        assert!(!root.path().join("report.json").exists());
    }

    #[test]
    fn rejects_unsupported_usage_without_side_effects() {
        assert!(run(vec![]).is_err());
        assert!(run(vec![OsString::from("pak"), OsString::from("extract")]).is_err());
    }

    #[test]
    fn collects_sorted_normalized_overlay_members() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("Libs").join("UI")).unwrap();
        fs::write(
            temp.path().join("Libs").join("UI").join("Widget.xml"),
            b"ui",
        )
        .unwrap();
        fs::write(temp.path().join("root.txt"), b"root").unwrap();
        let files = collect_overlay_files(temp.path()).unwrap();
        assert_eq!(files.1, 0);
        assert_eq!(
            files
                .0
                .iter()
                .map(|(member, _)| member.as_str())
                .collect::<Vec<_>>(),
            vec!["Libs/UI/Widget.xml", "root.txt"]
        );
    }

    #[test]
    fn refuses_empty_overlay_folder() {
        let temp = tempfile::tempdir().unwrap();
        assert!(collect_overlay_files(temp.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_overlay_entries() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("real.txt"), b"x").unwrap();
        symlink(temp.path().join("real.txt"), temp.path().join("link.txt")).unwrap();
        let (files, skipped_links) = collect_overlay_files(temp.path()).unwrap();
        assert_eq!(skipped_links, 1);
        assert_eq!(
            files
                .iter()
                .map(|(member, _)| member.as_str())
                .collect::<Vec<_>>(),
            vec!["real.txt"]
        );
    }

    #[test]
    fn refuses_overlay_folder_above_member_limit() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..=OVERLAY_MAX_MEMBERS {
            fs::write(temp.path().join(format!("file-{index}.txt")), b"x").unwrap();
        }
        assert!(collect_overlay_files(temp.path()).is_err());
    }
}
