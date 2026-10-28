// Esconde o console extra no Windows nas builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    window.run()
}
