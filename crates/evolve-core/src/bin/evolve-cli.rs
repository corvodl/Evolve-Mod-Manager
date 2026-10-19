//! Safe command-line functions shared with the desktop application.
use evolve_core::{
    export_dds_png, extract_legacy_pak, inspect_dds, inspect_legacy_pak, scan_directory,
    write_inventory_json,
};
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
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
        "  evolve-cli dds info <texture.dds|texture.dds.0> [dds-info.json]",
        "  evolve-cli dds png <texture.dds|texture.dds.0> <new-output.png>",
        "",
        "Stage 2 signed/encrypted CryPAKs cannot yet be extracted.",
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
    let output: Option<PathBuf> = args.next().map(Into::into);
    no_extra(&mut args)?;
    if verb == OsStr::new("pak") && subcommand == OsStr::new("inspect") {
        let result = inspect_legacy_pak(&source).map_err(|e| e.to_string())?;
        println!(
            "Legacy ZIP PAK: {} files, {} uncompressed bytes",
            result.entries.len(),
            result.total_bytes
        );
        for item in &result.entries {
            println!("  {:>10} {:>10} {}", item.size, item.compressed_size, item.path);
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
    } else if verb == OsStr::new("dds") && subcommand == OsStr::new("info") {
        let info = inspect_dds(&source).map_err(|e| e.to_string())?;
        println!(
            "{} x {}, {:?}, {} mip(s), {} stream part(s)",
            info.width, info.height, info.format, info.mipmaps, info.streaming_parts
        );
        if let Some(path) = output {
            write_report(&info, &path)?;
        }
    } else if verb == OsStr::new("dds") && subcommand == OsStr::new("png") {
        let target = output.ok_or_else(usage)?;
        let info = export_dds_png(&source, &target).map_err(|e| e.to_string())?;
        println!(
            "Exported original top mip {} x {} ({:?}) to {}",
            info.width, info.height, info.format, target.display()
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
