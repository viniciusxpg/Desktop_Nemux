//! A janela do app no Windows: saber se está em primeiro plano e piscar a barra de
//! tarefas quando um painel pede atenção. Sem janela de verdade (testes), a janela
//! conta como em primeiro plano e não pisca.

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

#[cfg(not(windows))]
pub fn is_foreground(_ui: &AppWindow) -> bool {
    true
}

#[cfg(not(windows))]
pub fn flash(_ui: &AppWindow) {}
