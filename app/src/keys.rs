//! Converte teclas do Slint nas sequências de bytes que um terminal espera.

/// Modificadores pressionados junto com a tecla.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    /// Parâmetro de modificador do xterm (`CSI 1;<m>A`); `None` sem modificadores.
    fn xterm_param(self) -> Option<u8> {
        let m = 1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8;
        (m > 1).then_some(m)
    }
}

/// Codifica uma tecla. `app_cursor` é o modo DECCKM, em que as setas usam `ESC O`.
/// Devolve `None` para teclas que não geram nada (ex.: Shift sozinho).
pub fn encode(text: &str, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        // Texto composto (IME, teclas mortas): envia como está.
        return Some(with_alt(text.as_bytes(), mods.alt));
    }

    let bytes: Vec<u8> = match c {
        // Shift, Control, Alt, AltGr, CapsLock, Meta (sozinhos).
        '\u{10}'..='\u{18}' => return None,
        '\u{8}' => vec![if mods.ctrl { 0x08 } else { 0x7f }],
        // Shift+Enter vira ESC CR (o mesmo que Alt+Enter): o ConPTY do Windows 10 descarta
        // as codificações modernas (CSI u, modifyOtherKeys), e o Claude Code entende ESC CR
        // como nova linha no prompt. No PowerShell não faz nada (não executa o comando).
        '\n' => with_alt(b"\r", mods.alt || mods.shift),
        '\t' if mods.shift => b"\x1b[Z".to_vec(),
        '\u{19}' => b"\x1b[Z".to_vec(),
        '\u{7f}' => tilde(3, mods),
        '\u{F700}' => cursor(b'A', mods, app_cursor),
        '\u{F701}' => cursor(b'B', mods, app_cursor),
        '\u{F702}' => cursor(b'D', mods, app_cursor),
        '\u{F703}' => cursor(b'C', mods, app_cursor),
        '\u{F729}' => cursor(b'H', mods, app_cursor),
        '\u{F72B}' => cursor(b'F', mods, app_cursor),
        '\u{F727}' => tilde(2, mods),
        '\u{F72C}' => tilde(5, mods),
        '\u{F72D}' => tilde(6, mods),
        '\u{F704}'..='\u{F707}' => {
            let letter = b'P' + (c as u32 - 0xF704) as u8;
            match mods.xterm_param() {
                Some(m) => format!("\x1b[1;{m}{}", letter as char).into_bytes(),
                None => vec![0x1b, b'O', letter],
            }
        }
        '\u{F708}'..='\u{F70F}' => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde(CODES[(c as u32 - 0xF708) as usize], mods)
        }
        // Demais teclas especiais do Slint (área de uso privado) não têm sequência.
        '\u{F700}'..='\u{F8FF}' => return None,
        // No Windows, AltGr chega como Ctrl+Alt: só trata como Ctrl se for uma letra.
        _ if mods.ctrl && (!mods.alt || c.is_ascii_alphabetic()) => {
            let code = ctrl_code(c)?;
            with_alt(&[code], mods.alt)
        }
        _ if mods.ctrl => text.as_bytes().to_vec(),
        _ => with_alt(text.as_bytes(), mods.alt),
    };
    Some(bytes)
}

fn ctrl_code(c: char) -> Option<u8> {
    match c {
        c if (c as u32) < 0x20 => Some(c as u8),
        'a'..='z' | 'A'..='Z' => Some(c.to_ascii_uppercase() as u8 & 0x1f),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn with_alt(bytes: &[u8], alt: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if alt {
        out.push(0x1b);
    }
    out.extend_from_slice(bytes);
    out
}

fn cursor(letter: u8, mods: Mods, app_cursor: bool) -> Vec<u8> {
    match mods.xterm_param() {
        Some(m) => format!("\x1b[1;{m}{}", letter as char).into_bytes(),
        None if app_cursor => vec![0x1b, b'O', letter],
        None => vec![0x1b, b'[', letter],
    }
}

fn tilde(code: u8, mods: Mods) -> Vec<u8> {
    match mods.xterm_param() {
        Some(m) => format!("\x1b[{code};{m}~").into_bytes(),
        None => format!("\x1b[{code}~").into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
    };
    const CTRL: Mods = Mods {
        ctrl: true,
        alt: false,
        shift: false,
    };

    fn enc(text: &str, mods: Mods) -> Vec<u8> {
        encode(text, mods, false).unwrap()
    }

    #[test]
    fn plain_text() {
        assert_eq!(enc("a", NONE), b"a");
        assert_eq!(enc("ç", NONE), "ç".as_bytes());
        assert_eq!(enc("\n", NONE), b"\r");
        assert_eq!(enc("\u{8}", NONE), b"\x7f");
    }

    #[test]
    fn shift_enter_is_esc_cr() {
        assert_eq!(
            enc(
                "\n",
                Mods {
                    shift: true,
                    ..NONE
                }
            ),
            b"\x1b\r"
        );
        assert_eq!(enc("\n", Mods { alt: true, ..NONE }), b"\x1b\r");
    }

    #[test]
    fn modifiers_alone_send_nothing() {
        assert_eq!(encode("\u{10}", NONE, false), None);
        assert_eq!(encode("\u{11}", CTRL, false), None);
        assert_eq!(encode("", NONE, false), None);
    }

    #[test]
    fn ctrl_letters() {
        assert_eq!(enc("c", CTRL), [0x03]);
        assert_eq!(enc("C", CTRL), [0x03]);
        assert_eq!(enc("[", CTRL), [0x1b]);
    }

    #[test]
    fn altgr_is_not_ctrl() {
        // AltGr+Q no teclado ABNT2 produz "/" com Ctrl+Alt.
        let altgr = Mods {
            ctrl: true,
            alt: true,
            shift: false,
        };
        assert_eq!(enc("/", altgr), b"/");
    }

    #[test]
    fn arrows_and_modes() {
        assert_eq!(enc("\u{F700}", NONE), b"\x1b[A");
        assert_eq!(encode("\u{F700}", NONE, true).unwrap(), b"\x1bOA");
        assert_eq!(enc("\u{F703}", CTRL), b"\x1b[1;5C");
    }

    #[test]
    fn special_keys() {
        assert_eq!(enc("\u{7f}", NONE), b"\x1b[3~");
        assert_eq!(enc("\u{F72C}", NONE), b"\x1b[5~");
        assert_eq!(enc("\u{F704}", NONE), b"\x1bOP");
        assert_eq!(enc("\u{F70F}", NONE), b"\x1b[24~");
        assert_eq!(
            enc(
                "\t",
                Mods {
                    shift: true,
                    ..NONE
                }
            ),
            b"\x1b[Z"
        );
    }
}
