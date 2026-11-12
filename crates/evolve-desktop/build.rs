use std::path::{Path, PathBuf};

fn main() {
    // Listing any rerun-if-changed path turns off cargo's default of
    // rerunning on every package change, so name every input.
    for input in ["build.rs", "app.manifest", "../../assets/hunt.ico"] {
        println!("cargo:rerun-if-changed={input}");
    }
    acknowledgements();
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-env-changed=EVOLVE_TEST_NO_MANIFEST");
        // Cargo test executables must remain runnable by an ordinary account;
        // the released GUI still embeds its normal elevation manifest.
        if std::env::var_os("EVOLVE_TEST_NO_MANIFEST").is_some() {
            return;
        }
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("../../assets/hunt.ico");
        // The private Qt manager requires UAC for protected game-file and
        // Windows Firewall workflows. Only this GUI executable embeds the
        // manifest: evolve-cli remains suitable for non-elevated workers.
        resource.set_manifest_file("app.manifest");
        resource.set("ProductName", "Evolve Mod Manager");
        resource.set("FileDescription", "Evolve Mod Manager");
        resource
            .compile()
            .expect("Application icon resource failed");
    }
}

/// Writes the crates the manager is built on (direct dependencies of the
/// desktop app and evolve-core) with their resolved version and licence,
/// for the Credits page. Only files are read: no network, no nested cargo.
/// Anything not found is reported as unknown rather than failing the build.
fn acknowledgements() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let core = manifest.join("../evolve-core/Cargo.toml");
    let lock = manifest.join("../../Cargo.lock");
    for path in [&manifest.join("Cargo.toml"), &core, &lock] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let mut names = Vec::<(String, String)>::new();
    for path in [manifest.join("Cargo.toml"), core] {
        for (name, requirement) in
            dependency_names(&std::fs::read_to_string(path).unwrap_or_default())
        {
            if !names.iter().any(|(known, _)| *known == name) {
                names.push((name, requirement));
            }
        }
    }
    names.sort();
    let locked = locked_versions(&std::fs::read_to_string(lock).unwrap_or_default());
    let registry = registry_sources();
    let mut out = String::from(
        "pub const CRATES: &[(&str, &str, &str)] = &[
",
    );
    for (name, requirement) in names {
        // Several versions of one crate can be locked; pick the newest
        // that this manifest's requirement accepts.
        let version = locked
            .iter()
            .filter(|(locked, version)| *locked == name && accepts(&requirement, version))
            .map(|(_, version)| version.clone())
            .max_by_key(|version| numeric(version))
            .unwrap_or_default();
        let license = registry
            .iter()
            .map(|root| root.join(format!("{name}-{version}")).join("Cargo.toml"))
            .find_map(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| package_license(&text))
            .unwrap_or_default();
        out.push_str(&format!(
            "    ({name:?}, {version:?}, {license:?}),
"
        ));
    }
    out.push_str(
        "];
",
    );
    let target = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let _ = std::fs::write(target.join("acknowledgements.rs"), out);
}

/// `[dependencies]` entries as (name, version requirement).
fn dependency_names(manifest: &str) -> Vec<(String, String)> {
    let mut names = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
            continue;
        }
        if !in_dependencies || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=')
            && !value.contains("path =")
        {
            let requirement = if value.trim_start().starts_with('"') {
                value.trim().trim_matches('"').to_owned()
            } else {
                value
                    .split("version = \"")
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .unwrap_or("")
                    .to_owned()
            };
            names.push((name.trim().to_owned(), requirement));
        }
    }
    names
}

fn numeric(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect()
}

/// Cargo's default (caret) and exact (`=`) requirements.
fn accepts(requirement: &str, version: &str) -> bool {
    let requirement = requirement.trim();
    if let Some(exact) = requirement.strip_prefix('=') {
        return exact.trim() == version;
    }
    let wanted = numeric(requirement.trim_start_matches('^'));
    let have = numeric(version);
    let Some(first) = wanted.iter().position(|part| *part != 0) else {
        return wanted.is_empty() || have.starts_with(&wanted);
    };
    // Everything up to and including the first non-zero part must match,
    // and the rest must be at least as new.
    have.len() >= wanted.len()
        && have[..=first] == wanted[..=first]
        && have[first + 1..wanted.len()] >= wanted[first + 1..]
}

fn locked_versions(lock: &str) -> Vec<(String, String)> {
    let mut packages = Vec::new();
    let mut name = None;
    for line in lock.lines() {
        if line.trim() == "[[package]]" {
            name = None;
        } else if let Some(value) = line.strip_prefix("name = ") {
            name = Some(value.trim_matches('"').to_owned());
        } else if let Some(value) = line.strip_prefix("version = ")
            && let Some(name) = name.take()
        {
            packages.push((name, value.trim_matches('"').to_owned()));
        }
    }
    packages
}

fn registry_sources() -> Vec<PathBuf> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| Path::new(&home).join(".cargo"))
        });
    let Some(home) = home else {
        return Vec::new();
    };
    std::fs::read_dir(home.join("registry").join("src"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn package_license(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && let Some(value) = line.strip_prefix("license = ") {
            return Some(value.trim_matches('"').to_owned());
        }
    }
    None
}
