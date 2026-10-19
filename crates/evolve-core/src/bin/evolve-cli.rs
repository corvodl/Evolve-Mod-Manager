use evolve_core::{scan_directory, write_inventory_json};
use std::path::PathBuf;

fn main() {
    if let Err(message) = run() {
        eprintln!("Error: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("scan")) {
        return Err("Usage: evolve-cli scan <local-directory> [output.json]".into());
    }
    let root: PathBuf = args.next().ok_or("Missing directory")?.into();
    let output: Option<PathBuf> = args.next().map(Into::into);
    if args.next().is_some() {
        return Err("Unexpected extra argument".into());
    }
    let inventory = scan_directory(&root).map_err(|e| e.to_string())?;
    println!(
        "Scanned {} assets ({} bytes); skipped {} links",
        inventory.entries.len(),
        inventory.total_bytes,
        inventory.skipped_symlinks
    );
    if let Some(output) = output {
        write_inventory_json(&inventory, &output).map_err(|e| e.to_string())?;
        println!("Report written to {}", output.display());
    }
    Ok(())
}
