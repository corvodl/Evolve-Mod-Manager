//! One-shot privileged worker foundation. No generic commands, scripts, or
//! filesystem-copy operations are exposed. A future UAC caller must verify
//! that this executable is installed in an administrator-controlled location.
#![cfg_attr(windows, windows_subsystem = "console")]

#[path = "../online_guard.rs"]
mod online_guard;
#[path = "../system_health.rs"]
mod system_health;
#[path = "../windows_system32.rs"]
mod windows_system32;

use std::path::{Component, Path, PathBuf};

const MAX_PATH_ARGUMENT: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Firewall {
        game: PathBuf,
        allow_online: bool,
    },
    Health {
        game: String,
        stage: String,
        projects: String,
    },
}

fn check_argument(value: &str, allow_empty: bool) -> Result<(), String> {
    if value.len() > MAX_PATH_ARGUMENT
        || value.chars().any(char::is_control)
        || (!allow_empty && value.is_empty())
    {
        return Err("Invalid or oversized worker path argument".into());
    }
    Ok(())
}

fn parse_request(args: &[String]) -> Result<Request, String> {
    match args {
        [operation, game] if operation == "firewall-block" => {
            check_argument(game, false)?;
            Ok(Request::Firewall {
                game: PathBuf::from(game),
                allow_online: false,
            })
        }
        [operation, game] if operation == "firewall-allow" => {
            check_argument(game, false)?;
            Ok(Request::Firewall {
                game: PathBuf::from(game),
                allow_online: true,
            })
        }
        [operation, game, stage, projects] if operation == "health" => {
            for value in [game, stage, projects] {
                check_argument(value, true)?;
            }
            Ok(Request::Health {
                game: game.clone(),
                stage: stage.clone(),
                projects: projects.clone(),
            })
        }
        _ => Err("Unsupported privileged operation or invalid argument count".into()),
    }
}

fn ensure_no_links(path: &Path) -> Result<(), String> {
    for parent in path.ancestors() {
        if parent.as_os_str().is_empty() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(parent)
            .map_err(|error| format!("Cannot verify protected path: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Linked paths are not allowed for privileged operations".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            // Windows junctions and other reparse points are not always
            // reported by std's is_symlink() predicate.
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Windows reparse-point paths are not allowed".into());
            }
        }
    }
    Ok(())
}

fn validate_game_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err("The selected EvolveGame path must be absolute and contain no '..'".into());
    }
    ensure_no_links(path)?;
    let directory = path.join("bin64_SteamRetail");
    ensure_no_links(&directory)?;
    let executable = directory.join("Evolve.exe");
    ensure_no_links(&executable)?;
    if !std::fs::metadata(&executable)
        .map_err(|error| format!("Cannot verify Evolve.exe: {error}"))?
        .is_file()
    {
        return Err("The selected game executable is not a regular file".into());
    }
    std::fs::canonicalize(path)
        .map_err(|error| format!("Cannot canonicalize selected game root: {error}"))
}

fn execute(request: Request) -> Result<String, String> {
    match request {
        Request::Firewall { game, allow_online } => {
            let verified = validate_game_root(&game)?;
            online_guard::apply(&verified, allow_online)
        }
        Request::Health {
            game,
            stage,
            projects,
        } => {
            // Health is purely observational, with no registry, firewall,
            // backup, or other writes, even if started after a UAC prompt.
            let snapshot = system_health::inspect(&game, &stage, &projects);
            serde_json::to_string(&serde_json::json!({
                "firewall": snapshot.firewall,
                "game_process": snapshot.game_process,
                "storage": snapshot.storage,
                "dumps": snapshot.dumps,
                "environment": snapshot.environment,
                "recovery": snapshot.recovery
            }))
            .map_err(|error| error.to_string())
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = parse_request(&args).and_then(execute);
    match outcome {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("Privileged worker refused operation: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_operations_and_argument_counts_only() {
        assert!(matches!(
            parse_request(&["firewall-block".into(), "/game".into()]),
            Ok(Request::Firewall {
                allow_online: false,
                ..
            })
        ));
        assert!(matches!(
            parse_request(&["firewall-allow".into(), "/game".into()]),
            Ok(Request::Firewall {
                allow_online: true,
                ..
            })
        ));
        assert!(matches!(
            parse_request(&["health".into(), "".into(), "".into(), "".into()]),
            Ok(Request::Health { .. })
        ));
        for inputs in [
            vec!["run".into(), "cmd.exe".into()],
            vec!["restore".into(), "/game".into()],
            vec!["firewall-block".into()],
            vec!["firewall-block".into(), "/game".into(), "extra".into()],
            vec!["health".into(), "/game".into()],
        ] {
            assert!(parse_request(&inputs).is_err(), "{inputs:?}");
        }
    }

    #[test]
    fn malicious_or_oversized_arguments_fail_closed() {
        assert!(parse_request(&["firewall-allow".into(), "".into()]).is_err());
        assert!(parse_request(&["firewall-allow".into(), "x".repeat(4097)]).is_err());
        assert!(parse_request(&["health".into(), "a\nb".into(), "".into(), "".into()]).is_err());
    }

    #[test]
    fn game_root_must_exist_and_resolve_only_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("EvolveGame");
        let folder = root.join("bin64_SteamRetail");
        std::fs::create_dir_all(&folder).unwrap();
        assert!(validate_game_root(&root).is_err());
        std::fs::write(folder.join("Evolve.exe"), b"not a real game").unwrap();
        assert_eq!(
            validate_game_root(&root).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );
        assert!(validate_game_root(Path::new("relative")).is_err());
        assert!(validate_game_root(&root.join("..")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_game_executable_is_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("EvolveGame");
        let folder = root.join("bin64_SteamRetail");
        std::fs::create_dir_all(&folder).unwrap();
        let other = temp.path().join("other.exe");
        std::fs::write(&other, b"other").unwrap();
        symlink(other, folder.join("Evolve.exe")).unwrap();
        assert!(validate_game_root(&root).is_err());
    }
}
