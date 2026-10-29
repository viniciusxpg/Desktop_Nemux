//! Configuração do usuário: `%APPDATA%\nemux\config.toml` (ou `NEMUX_CONFIG`).
//! Tudo é opcional; o que faltar fica com o padrão. Lida só ao abrir o app.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use slint::ComponentHandle;

use crate::terminal::{Colors, Rgb};
use crate::{AppWindow, Theme};

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Linha de comando de cada painel novo, ex.: `"pwsh.exe -NoLogo"`.
    pub shell: Option<String>,
    pub font: Font,
    /// Cores do terminal.
    pub colors: TermColors,
    /// Cores da interface em volta dos terminais.
    pub app: AppColors,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    pub family: Option<String>,
    pub size: Option<f32>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TermColors {
    pub foreground: Option<Hex>,
    pub background: Option<Hex>,
    pub selection: Option<Hex>,
    /// As 16 cores ANSI: preto, vermelho, verde, amarelo, azul, magenta, ciano,
    /// branco e as 8 versões claras, nessa ordem.
    pub palette: Option<Vec<Hex>>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AppColors {
    pub tab_bar: Option<Hex>,
    pub tab_hover: Option<Hex>,
    pub accent: Option<Hex>,
    pub muted: Option<Hex>,
    pub divider: Option<Hex>,
}

/// Cor escrita como `"#rrggbb"` ou `"#rrggbbaa"`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub struct Hex(pub u8, pub u8, pub u8, pub u8);

impl TryFrom<String> for Hex {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        let err = || format!("cor inválida {s:?}: use \"#rrggbb\" ou \"#rrggbbaa\"");
        let hex = s.strip_prefix('#').ok_or_else(err)?;
        if !matches!(hex.len(), 6 | 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(err());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        let alpha = if hex.len() == 8 { byte(6) } else { 0xff };
        Ok(Hex(byte(0), byte(2), byte(4), alpha))
    }
}

impl Hex {
    fn rgb(self) -> Rgb {
        (self.0, self.1, self.2)
    }

    fn color(self) -> slint::Color {
        slint::Color::from_argb_u8(self.3, self.0, self.1, self.2)
    }
}

/// Onde o app procura a configuração.
pub fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("NEMUX_CONFIG") {
        return path.into();
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nemux")
        .join("config.toml")
}

impl Config {
    /// Lê `path`; um arquivo que não existe vale como configuração vazia.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Self = toml::from_str(text).map_err(|e| e.message().to_owned())?;
        if let Some(palette) = &config.colors.palette
            && palette.len() != 16
        {
            return Err(format!(
                "colors.palette precisa de 16 cores (tem {})",
                palette.len()
            ));
        }
        if let Some(size) = config.font.size
            && !(6.0..=72.0).contains(&size)
        {
            return Err(format!("font.size precisa estar entre 6 e 72 (é {size})"));
        }
        Ok(config)
    }

    /// Cores do terminal, com o padrão no que não foi configurado.
    pub fn terminal_colors(&self) -> Colors {
        let mut colors = Colors::default();
        let c = &self.colors;
        if let Some(fg) = c.foreground {
            colors.foreground = fg.rgb();
        }
        if let Some(bg) = c.background {
            colors.background = bg.rgb();
        }
        if let Some(palette) = &c.palette {
            for (slot, hex) in colors.palette.iter_mut().zip(palette) {
                *slot = hex.rgb();
            }
        }
        colors
    }

    /// Passa fonte e cores para o tema da interface.
    pub fn apply_theme(&self, ui: &AppWindow) {
        let theme = ui.global::<Theme>();
        let colors = self.terminal_colors();
        let rgb = |(r, g, b): Rgb| slint::Color::from_rgb_u8(r, g, b);
        // O fundo e o texto do painel são os do terminal; a aba ativa se funde com ele.
        theme.set_background(rgb(colors.background));
        theme.set_tab_active(rgb(colors.background));
        theme.set_foreground(rgb(colors.foreground));
        if let Some(c) = self.colors.selection {
            theme.set_selection(c.color());
        }
        let app = &self.app;
        if let Some(c) = app.tab_bar {
            theme.set_tab_bar(c.color());
        }
        if let Some(c) = app.tab_hover {
            theme.set_tab_hover(c.color());
        }
        if let Some(c) = app.accent {
            theme.set_accent(c.color());
        }
        if let Some(c) = app.muted {
            theme.set_muted(c.color());
        }
        if let Some(c) = app.divider {
            theme.set_divider(c.color());
        }
        if let Some(family) = &self.font.family {
            theme.set_font_family(family.as_str().into());
        }
        if let Some(size) = self.font.size {
            theme.set_font_size(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vazio_usa_o_padrao() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::default().terminal_colors(), Colors::default());
    }

    #[test]
    fn le_tudo() {
        let config = Config::parse(
            r##"
            shell = "pwsh.exe -NoLogo"
            [font]
            family = "Cascadia Mono"
            size = 13
            [colors]
            foreground = "#ffffff"
            background = "#000000"
            selection = "#ff000080"
            palette = ["#000001", "#000002", "#000003", "#000004", "#000005", "#000006",
                       "#000007", "#000008", "#000009", "#00000a", "#00000b", "#00000c",
                       "#00000d", "#00000e", "#00000f", "#000010"]
            [app]
            accent = "#00ff00"
            "##,
        )
        .unwrap();
        assert_eq!(config.shell.as_deref(), Some("pwsh.exe -NoLogo"));
        assert_eq!(config.font.family.as_deref(), Some("Cascadia Mono"));
        assert_eq!(config.font.size, Some(13.0));
        assert_eq!(config.colors.selection, Some(Hex(0xff, 0, 0, 0x80)));
        assert_eq!(config.app.accent, Some(Hex(0, 0xff, 0, 0xff)));
        let colors = config.terminal_colors();
        assert_eq!(colors.foreground, (0xff, 0xff, 0xff));
        assert_eq!(colors.background, (0, 0, 0));
        assert_eq!(colors.palette[0], (0, 0, 1));
        assert_eq!(colors.palette[15], (0, 0, 0x10));
    }

    /// O exemplo do README é válido e mostra as cores padrão.
    #[test]
    fn exemplo_do_readme() {
        let readme = include_str!("../../README.md").replace("\r\n", "\n");
        let exemplo = readme
            .split("```toml\n")
            .nth(1)
            .and_then(|s| s.split("```").next())
            .expect("bloco toml no README");
        let config = Config::parse(exemplo).unwrap();
        assert_eq!(config.terminal_colors(), Colors::default());
    }

    #[test]
    fn erros_dizem_o_que_esta_errado() {
        let erro = Config::parse("[colors]\nforeground = \"branco\"").unwrap_err();
        assert!(erro.contains("cor inválida \"branco\""), "{erro}");
        let erro = Config::parse("[colors]\npalette = [\"#000000\"]").unwrap_err();
        assert!(erro.contains("16 cores"), "{erro}");
        let erro = Config::parse("[font]\nsize = 200").unwrap_err();
        assert!(erro.contains("entre 6 e 72"), "{erro}");
        let erro = Config::parse("fonte = 1").unwrap_err();
        assert!(erro.contains("fonte"), "{erro}");
    }

    #[test]
    fn arquivo_que_nao_existe_e_configuracao_vazia() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        std::fs::write(&path, "shell = 1").unwrap();
        let erro = Config::load(&path).unwrap_err();
        assert!(erro.starts_with(&path.display().to_string()), "{erro}");
    }
}
