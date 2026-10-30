//! A janela do app no Windows: saber se está em primeiro plano, piscar a barra de
//! tarefas quando um painel pede atenção e onde está o ponteiro (arquivos soltos).
//! Sem janela de verdade (testes), a janela conta como em primeiro plano, não pisca
//! e não sabe do ponteiro.

#[cfg(windows)]
use slint::ComponentHandle;

use crate::AppWindow;

#[cfg(windows)]
fn hwnd(ui: &AppWindow) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = ui.window().window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as _),
        _ => None,
    }
}

#[cfg(windows)]
pub fn is_foreground(ui: &AppWindow) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    // SAFETY: só compara o handle da janela da frente com o nosso.
    hwnd(ui).is_none_or(|hwnd| unsafe { GetForegroundWindow() } == hwnd)
}

/// Pisca o botão na barra de tarefas até a janela vir para a frente; se ela já está
/// na frente, o Windows não faz nada.
#[cfg(windows)]
pub fn flash(ui: &AppWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx,
    };

    let Some(hwnd) = hwnd(ui) else {
        return;
    };
    let info = FLASHWINFO {
        cbSize: size_of::<FLASHWINFO>() as u32,
        hwnd,
        dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
        uCount: 0,
        dwTimeout: 0,
    };
    // SAFETY: `info` vive durante a chamada e o handle é o da nossa janela.
    unsafe { FlashWindowEx(&info) };
}

/// Posição do ponteiro na janela, em pixels lógicos. Durante o arrastar e soltar de
/// arquivos o winit não manda o movimento do ponteiro, então ela é lida do Windows.
#[cfg(windows)]
pub fn cursor_position(ui: &AppWindow) -> Option<(f32, f32)> {
    use slint::winit_030::WinitWindowAccessor;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let origin = ui
        .window()
        .with_winit_window(|w| w.inner_position().ok())??;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: `p` vive durante a chamada.
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return None;
    }
    let scale = ui.window().scale_factor();
    Some((
        (p.x - origin.x) as f32 / scale,
        (p.y - origin.y) as f32 / scale,
    ))
}

#[cfg(not(windows))]
pub fn cursor_position(_ui: &AppWindow) -> Option<(f32, f32)> {
    None
}

#[cfg(not(windows))]
pub fn is_foreground(_ui: &AppWindow) -> bool {
    true
}

#[cfg(not(windows))]
pub fn flash(_ui: &AppWindow) {}
