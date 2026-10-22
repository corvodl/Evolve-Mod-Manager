//! Local-only HTTP bridge for Ruffle Web's live ExternalInterface API.
//! Only preverified original menu handlers can be queued from GFX Tools.
//! No native CryEngine callbacks, game process, file writes, or arbitrary URLs.
use crate::{gfx_bundled_tools, gfx_interactive::PreviewPreset};
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_REQUEST: usize = 12 * 1024;
const MAX_STATIC_ASSET: u64 = 64 * 1024 * 1024;
const MAX_SWF_ASSET: u64 = 40 * 1024 * 1024;

pub struct LiveServer {
    active: Arc<AtomicBool>,
    queued: mpsc::Sender<PreviewPreset>,
    supported: Vec<PreviewPreset>,
    url: String,
    worker: Option<JoinHandle<()>>,
    _scratch: tempfile::TempDir,
}

impl std::fmt::Debug for LiveServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveServer")
            .field("supported", &self.supported)
            .finish()
    }
}

impl LiveServer {
    pub fn start(
        scratch: tempfile::TempDir,
        supported: Vec<PreviewPreset>,
        runtime: &str,
    ) -> Result<Self, String> {
        // This is the pin-verified Ruffle WASM bundle shipped with GFX Tools.
        // Desktop Ruffle does not implement browser-side ExternalInterface RPC.
        let app = std::env::current_exe().map_err(|e| e.to_string())?;
        let app_folder = app.parent().ok_or("No executable directory")?;
        let js = gfx_bundled_tools::detected_ruffle_web(runtime, app_folder)
            .ok_or_else(|| {
                if !runtime.trim().is_empty() {
                    "Selected Ruffle Web path is incomplete or invalid. Settings → Integrations → Ruffle Web needs ruffle.js and its adjacent .wasm file.".to_string()
                } else {
                    "Ruffle Web is not installed. Go to Settings → Integrations → Ruffle → Install Ruffle Web, or select an existing ruffle.js with its adjacent .wasm file.".to_string()
                }
            })?;
        let bundle = js
            .parent()
            .ok_or("Ruffle Web ruffle.js has no runtime folder")?
            .to_path_buf();
        let token = scratch
            .path()
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Live GFX preview has no session token")?
            .to_owned();
        if token.len() < 16
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("Invalid random live GFX preview session token".into());
        }
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| format!("Unable to bind isolated local GFX preview: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let (queued, receiver) = mpsc::channel();
        let active = Arc::new(AtomicBool::new(true));
        let live = Arc::clone(&active);
        let files = scratch.path().to_path_buf();
        let worker = thread::Builder::new()
            .name("isolated-gfx-web-preview".into())
            .spawn(move || serve(listener, files, bundle, token, receiver, live))
            .map_err(|e| e.to_string())?;
        let url = format!(
            "http://127.0.0.1:{port}/{}/",
            scratch
                .path()
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Missing preview token")?
        );
        if let Err(error) = open_browser(&url) {
            active.store(false, Ordering::Release);
            let _ = worker.join();
            return Err(error);
        }
        Ok(Self {
            active,
            queued,
            supported,
            url,
            worker: Some(worker),
            _scratch: scratch,
        })
    }

    pub fn is_running(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub fn reopen(&self) -> Result<(), String> {
        if !self.is_running() {
            return Err("Live GFX preview is stopped".into());
        }
        open_browser(&self.url)
    }
    pub fn supports(&self, preset: PreviewPreset) -> bool {
        preset == PreviewPreset::AsAuthored || self.supported.contains(&preset)
    }
    pub fn dispatch(&self, preset: PreviewPreset) -> Result<(), String> {
        if !self.is_running() {
            return Err("Live preview is closed; reopen it to continue".into());
        }
        if !self.supports(preset) {
            return Err(format!(
                "{} is not registered by this original GFX; no action was sent.",
                preset.description()
            ));
        }
        self.queued
            .send(preset)
            .map_err(|_| "Live preview bridge disconnected".into())
    }
}
impl Drop for LiveServer {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn open_browser(url: &str) -> Result<(), String> {
    let (bin, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        ("rundll32.exe", vec!["url.dll,FileProtocolHandler", url])
    } else if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else {
        ("xdg-open", vec![url])
    };
    Command::new(bin)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Cannot open local Ruffle Web preview: {e}"))
}
fn serve(
    listener: TcpListener,
    root: PathBuf,
    bundle: PathBuf,
    token: String,
    receiver: mpsc::Receiver<PreviewPreset>,
    active: Arc<AtomicBool>,
) {
    while active.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, addr)) => {
                if !addr.ip().is_loopback() {
                    continue;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_millis(650)));
                // Ruffle Web WASM binaries can be several MiB; don't truncate slow local loads.
                let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
                respond(&mut stream, &root, &bundle, &token, &receiver);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
    active.store(false, Ordering::Release);
}
fn respond(
    stream: &mut TcpStream,
    root: &Path,
    bundle: &Path,
    token: &str,
    receiver: &mpsc::Receiver<PreviewPreset>,
) {
    let mut buffer = [0u8; MAX_REQUEST];
    let Ok(amount) = stream.read(&mut buffer) else {
        return;
    };
    if amount == 0 {
        return;
    }
    let Ok(header) = std::str::from_utf8(&buffer[..amount]) else {
        send(stream, 400, "text/plain", b"Invalid request");
        return;
    };
    let Some(first) = header.lines().next() else {
        return;
    };
    let Some((method, rest)) = first.split_once(' ') else {
        return;
    };
    let Some((uri, protocol)) = rest.split_once(' ') else {
        return;
    };
    if method != "GET" || protocol.trim_end() != "HTTP/1.1" && protocol.trim_end() != "HTTP/1.0" {
        send(
            stream,
            405,
            "text/plain",
            b"Only local GET preview requests are supported",
        );
        return;
    }
    let prefix = format!("/{token}/");
    let Some(relative) = uri.strip_prefix(&prefix) else {
        send(stream, 404, "text/plain", b"Preview session not found");
        return;
    };
    if relative.contains(['?', '#', '%', '\\'])
        || relative.starts_with('/')
        || relative.contains("..")
        || relative.contains(':')
    {
        send(stream, 400, "text/plain", b"Invalid preview path");
        return;
    }
    match relative {
        "" | "index.html" => send(
            stream,
            200,
            "text/html; charset=utf-8",
            include_bytes!("gfx_live_player.html"),
        ),
        "preview.js" => send(
            stream,
            200,
            "application/javascript; charset=utf-8",
            include_bytes!("gfx_live_player.js"),
        ),
        "preview-menu.json" => {
            // Only the generated same-build UI_Data manifest is visible:
            // never expose the editor's source files or the user's filesystem.
            serve_file(
                stream,
                &root.join("preview-menu.json"),
                root,
                64 * 1024,
                "application/json; charset=utf-8",
            );
        }
        "next" => {
            let answer = match receiver.try_recv() {
                Ok(PreviewPreset::AsAuthored) => json!({"kind":"reload"}),
                Ok(preset) => {
                    let (function, argument) = preset.script_call().expect("verified preset");
                    json!({"kind":"call",
                           "label":preset.description(),
                           "name":format!("evolvePreview_{function}"),
                           "arg":argument})
                }
                Err(_) => json!({"kind":"idle"}),
            };
            let payload = serde_json::to_vec(&answer).unwrap_or_default();
            send(stream, 200, "application/json", &payload);
        }
        path if path.starts_with("ruffle/") => {
            let filename = &path[7..];
            if !safe_asset(filename) {
                send(stream, 400, "text/plain", b"Unsafe runtime asset");
                return;
            }
            let suffix = filename.rsplit('.').next().unwrap_or("");
            let mime = match suffix {
                "js" => "application/javascript",
                "wasm" => "application/wasm",
                "json" => "application/json",
                "map" => "application/json",
                _ => "application/octet-stream",
            };
            serve_file(
                stream,
                &bundle.join(filename),
                bundle,
                MAX_STATIC_ASSET,
                mime,
            );
        }
        filename if filename == "movie.swf" || safe_import_asset(filename) => {
            serve_file(
                stream,
                &root.join(filename),
                root,
                MAX_SWF_ASSET,
                "application/x-shockwave-flash",
            );
        }
        _ => send(stream, 404, "text/plain", b"Asset unavailable"),
    }
}
fn safe_asset(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 160
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && !name.starts_with('.')
        && !name.contains("..")
        && (name.ends_with(".js")
            || name.ends_with(".wasm")
            || name.ends_with(".json")
            || name.ends_with(".map"))
}
fn safe_import_asset(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 1024
        && !name.contains(['\\', ':', '%', '?', '#'])
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && (name.ends_with(".swf") || name.ends_with(".gfx"))
}
fn serve_file(stream: &mut TcpStream, path: &Path, base: &Path, limit: u64, mime: &str) {
    let Ok(real) = fs::canonicalize(path) else {
        send(stream, 404, "text/plain", b"Preview file missing");
        return;
    };
    // On Windows canonicalize() returns the verbatim \\?\\ form, while
    // TempDir::path() may be a regular drive path. Compare like with like.
    let Ok(canonical_base) = fs::canonicalize(base) else {
        send(stream, 404, "text/plain", b"Preview root unavailable");
        return;
    };
    if !real.starts_with(&canonical_base) {
        send(stream, 403, "text/plain", b"Asset outside preview root");
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.file_type().is_file() || metadata.len() > limit {
        send(
            stream,
            403,
            "text/plain",
            b"Preview asset is not a bounded file",
        );
        return;
    }
    match fs::read(real) {
        Ok(data) => send(stream, 200, mime, &data),
        Err(_) => send(stream, 404, "text/plain", b"Unable to read preview file"),
    }
}
fn send(stream: &mut TcpStream, code: u16, mime: &str, payload: &[u8]) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; worker-src 'self' blob:; object-src 'none'; frame-ancestors 'none'\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_remote_or_unbounded_asset_paths() {
        assert!(safe_asset("ruffle.js"));
        assert!(safe_asset("core.ruffle.ab12.wasm"));
        for path in [
            "../x.js",
            "secret.txt",
            "foo/ruffle.js",
            ".env",
            "C:.js",
            "a%2e.js",
        ] {
            assert!(!safe_asset(path), "{path}");
        }
        assert!(safe_import_asset("effects/other.swf"));
        assert!(!safe_import_asset("../private.swf"));
        assert!(!safe_import_asset("C:/test.swf"));
    }
    #[test]
    fn loopback_server_serves_only_its_session_and_queued_actions() {
        fn get(path: &str, action: Option<PreviewPreset>) -> String {
            let src = tempfile::tempdir().unwrap();
            let web = tempfile::tempdir().unwrap();
            fs::write(src.path().join("movie.swf"), b"FWSsynthetic").unwrap();
            fs::write(web.path().join("ruffle.js"), b"// bundled preview").unwrap();
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let destination = listener.local_addr().unwrap();
            let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
            let source = src.path().to_path_buf();
            let bundle = web.path().to_path_buf();
            let reply = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let (sender, received) = mpsc::channel();
                if let Some(preset) = action {
                    sender.send(preset).unwrap();
                }
                respond(&mut socket, &source, &bundle, "private-token", &received);
            });
            let mut client = TcpStream::connect(destination).unwrap();
            client.write_all(request.as_bytes()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            let mut bytes = Vec::new();
            client.read_to_end(&mut bytes).unwrap();
            reply.join().unwrap();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        let page = get("/private-token/", None);
        assert!(page.starts_with("HTTP/1.1 200 OK"));
        assert!(page.contains("Live GFX Preview"));
        let callback = get("/private-token/next", Some(PreviewPreset::HideEngagement));
        assert!(callback.contains(r#""arg":false"#));
        assert!(get("/other-session/movie.swf", None).contains("404 Not Found"));
        assert!(get("/private-token/../test.swf", None).contains("400 Bad Request"));
        assert!(get("/private-token/ruffle/ruffle.js", None).contains("// bundled preview"));
        assert!(get("/private-token/preview-menu.json", None).contains("404 Not Found"));
    }

    #[test]
    fn preview_queue_rejects_unknown_source_handlers() {
        let (sender, rx) = mpsc::channel();
        let state = LiveServer {
            active: Arc::new(AtomicBool::new(true)),
            queued: sender,
            supported: vec![PreviewPreset::Back],
            url: "http://127.0.0.1:0/private-token/".into(),
            worker: None,
            _scratch: tempfile::tempdir().unwrap(),
        };
        assert!(!state.supports(PreviewPreset::HideSigningIn));
        assert!(state.dispatch(PreviewPreset::HideSigningIn).is_err());
        state.dispatch(PreviewPreset::Back).unwrap();
        assert_eq!(rx.try_recv().unwrap(), PreviewPreset::Back);
        assert!(state.supports(PreviewPreset::AsAuthored));
    }
}
