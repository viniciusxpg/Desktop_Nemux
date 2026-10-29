//! Configuração do usuário: `%APPDATA%\nemux\config.toml` (ou `NEMUX_CONFIG`).
//! Tudo é opcional; o que faltar fica com o padrão. Lida ao abrir o app; o tema
//! também pode ser trocado pela interface, que o grava de volta no arquivo.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::terminal::{Colors, Rgb};
use crate::{AppWindow, Theme, ThemeItem};

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Linha de comando de cada painel novo, ex.: `"pwsh.exe -NoLogo"`.
    pub shell: Option<String>,
    /// Nome de um dos temas prontos (`themes.toml`); sem ele, o primeiro.
    pub theme: Option<String>,
    pub font: Font,
    /// Cores do terminal, por cima das do tema.
    pub colors: TermColors,
    /// Cores da interface em volta dos terminais, por cima das do tema.
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

    pub fn color(self) -> slint::Color {
        slint::Color::from_argb_u8(self.3, self.0, self.1, self.2)
    }
}

/// Um tema pronto, como está em `themes.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub name: String,
    pub colors: TermColors,
    pub app: AppColors,
}

/// Os temas prontos; o primeiro é o padrão.
pub fn presets() -> &'static [Preset] {
    #[derive(Deserialize)]
    struct File {
        theme: Vec<Preset>,
    }
    static PRESETS: OnceLock<Vec<Preset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        let file: File = toml::from_str(include_str!("themes.toml")).expect("themes.toml válido");
        file.theme
    })
}

/// Todas as cores já resolvidas: o tema com as do usuário por cima.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub terminal: Colors,
    pub selection: Hex,
    pub tab_bar: Hex,
    pub tab_hover: Hex,
    pub accent: Hex,
    pub muted: Hex,
    pub divider: Hex,
}

impl Scheme {
    /// O tema `preset`; os testes garantem que todo tema pronto define todas as cores.
    fn of(preset: &Preset) -> Self {
        let (c, a) = (&preset.colors, &preset.app);
        let mut scheme = Self {
            terminal: Colors::default(),
            selection: c.selection.expect("tema sem selection"),
            tab_bar: a.tab_bar.expect("tema sem tab_bar"),
            tab_hover: a.tab_hover.expect("tema sem tab_hover"),
            accent: a.accent.expect("tema sem accent"),
            muted: a.muted.expect("tema sem muted"),
            divider: a.divider.expect("tema sem divider"),
        };
        scheme.overlay(c, a);
        scheme
    }

    /// Troca as cores que `colors` e `app` definem.
    fn overlay(&mut self, colors: &TermColors, app: &AppColors) {
        let t = &mut self.terminal;
        if let Some(c) = colors.foreground {
            t.foreground = c.rgb();
        }
        if let Some(c) = colors.background {
            t.background = c.rgb();
        }
        if let Some(palette) = &colors.palette {
            for (slot, hex) in t.palette.iter_mut().zip(palette) {
                *slot = hex.rgb();
            }
        }
        let pairs = [
            (&mut self.selection, colors.selection),
            (&mut self.tab_bar, app.tab_bar),
            (&mut self.tab_hover, app.tab_hover),
            (&mut self.accent, app.accent),
            (&mut self.muted, app.muted),
            (&mut self.divider, app.divider),
        ];
        for (slot, color) in pairs {
            if let Some(c) = color {
                *slot = c;
            }
        }
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
        if let Some(name) = &config.theme
            && preset_index(name).is_none()
        {
            let names: Vec<_> = presets().iter().map(|p| p.name.as_str()).collect();
            return Err(format!(
                "tema desconhecido {name:?} (os temas são: {})",
                names.join(", ")
            ));
        }
        Ok(config)
    }

    /// Índice do tema escolhido em `presets()`.
    pub fn theme_index(&self) -> usize {
        self.theme.as_deref().and_then(preset_index).unwrap_or(0)
    }

    /// O arquivo troca alguma cor do tema (em `[colors]` ou `[app]`)?
    pub fn overrides_theme(&self) -> bool {
        self.colors != TermColors::default() || self.app != AppColors::default()
    }

    pub fn scheme(&self) -> Scheme {
        let mut scheme = Scheme::of(&presets()[self.theme_index()]);
        scheme.overlay(&self.colors, &self.app);
        scheme
    }

    /// Cores do terminal, já com o tema.
    pub fn terminal_colors(&self) -> Colors {
        self.scheme().terminal
    }

    /// Passa fonte e cores para o tema da interface.
    pub fn apply_theme(&self, ui: &AppWindow) {
        let theme = ui.global::<Theme>();
        let scheme = self.scheme();
        let rgb = |(r, g, b): Rgb| slint::Color::from_rgb_u8(r, g, b);
        // O fundo e o texto do painel são os do terminal; a aba ativa se funde com ele.
        theme.set_background(rgb(scheme.terminal.background));
        theme.set_tab_active(rgb(scheme.terminal.background));
        theme.set_foreground(rgb(scheme.terminal.foreground));
        theme.set_selection(scheme.selection.color());
        theme.set_tab_bar(scheme.tab_bar.color());
        theme.set_tab_hover(scheme.tab_hover.color());
        theme.set_accent(scheme.accent.color());
        theme.set_muted(scheme.muted.color());
        theme.set_divider(scheme.divider.color());
        if let Some(family) = &self.font.family {
            theme.set_font_family(family.as_str().into());
        }
        if let Some(size) = self.font.size {
            theme.set_font_size(size);
        }
    }
}

/// Os temas prontos como a lista da interface os mostra.
pub fn theme_items() -> Vec<ThemeItem> {
    presets()
        .iter()
        .map(|preset| {
            let scheme = Scheme::of(preset);
            let rgb = |(r, g, b): Rgb| slint::Color::from_rgb_u8(r, g, b);
            // Vermelho, verde, amarelo, azul, magenta e ciano.
            let swatches: Vec<_> = scheme.terminal.palette[1..7]
                .iter()
                .map(|&c| rgb(c))
                .collect();
            ThemeItem {
                name: preset.name.as_str().into(),
                background: rgb(scheme.terminal.background),
                foreground: rgb(scheme.terminal.foreground),
                accent: scheme.accent.color(),
                swatches: ModelRc::new(VecModel::from(swatches)),
            }
        })
        .collect()
}

fn preset_index(name: &str) -> Option<usize> {
    presets()
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name))
}

/// Grava `theme = "<name>"` em `path`, mantendo o resto do arquivo (e os comentários).
pub fn save_theme(path: &Path, name: &str) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(err(&e)),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| err(&e))?;
    doc["theme"] = toml_edit::value(name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| err(&e))?;
    }
    std::fs::write(path, doc.to_string()).map_err(|e| err(&e))
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
    fn temas_prontos_definem_todas_as_cores() {
        assert!(presets().len() > 1);
        for preset in presets() {
            let (c, a) = (&preset.colors, &preset.app);
            assert!(
                c.foreground.is_some() && c.background.is_some(),
                "{}",
                preset.name
            );
            assert_eq!(
                c.palette.as_ref().map(Vec::len),
                Some(16),
                "{}",
                preset.name
            );
            Scheme::of(preset);
            assert!(a.tab_bar.is_some() && a.muted.is_some(), "{}", preset.name);
        }
        // O primeiro é o padrão do terminal (e do theme.slint).
        assert_eq!(Scheme::of(&presets()[0]).terminal, Colors::default());
    }

    #[test]
    fn tema_e_cores_por_cima() {
        let config = Config::parse("theme = \"dracula\"").unwrap();
        assert_eq!(presets()[config.theme_index()].name, "Dracula");
        assert_eq!(config.terminal_colors().background, (0x28, 0x2a, 0x36));
        assert!(!config.overrides_theme());

        let config =
            Config::parse("theme = \"Dracula\"\n[colors]\nbackground = \"#000000\"").unwrap();
        let scheme = config.scheme();
        assert_eq!(scheme.terminal.background, (0, 0, 0));
        assert_eq!(
            scheme.terminal.foreground,
            (0xf8, 0xf8, 0xf2),
            "o resto é do tema"
        );
        assert!(config.overrides_theme());

        let erro = Config::parse("theme = \"Nada\"").unwrap_err();
        assert!(erro.contains("tema desconhecido \"Nada\""), "{erro}");
        assert!(erro.contains("Dracula"), "{erro}");
    }

    #[test]
    fn gravar_o_tema_mantem_o_resto() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        save_theme(&path, "Dracula").unwrap();
        assert_eq!(
            Config::load(&path).unwrap().theme.as_deref(),
            Some("Dracula")
        );

        let original =
            "# meu shell\nshell = \"cmd.exe\"\ntheme = \"Nemux\"\n\n[font]\nsize = 12 # pequeno\n";
        std::fs::write(&path, original).unwrap();
        save_theme(&path, "Tokyo Night").unwrap();
        let texto = std::fs::read_to_string(&path).unwrap();
        assert_eq!(texto, original.replace("\"Nemux\"", "\"Tokyo Night\""));
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
