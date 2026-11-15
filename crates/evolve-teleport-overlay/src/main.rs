//! External, optional crosshair-marker overlay for windowed-fullscreen Evolve.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    evolve_teleport_overlay::run();
}
