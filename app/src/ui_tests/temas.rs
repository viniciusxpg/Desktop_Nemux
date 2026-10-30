// Lista de temas prontos (engrenagem → "Temas"): aplica na hora e grava no config.toml.

use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, Model};

use super::config::abrir_com_config;
use super::{achar, capturar, cmd, esperar_texto};
use crate::{AppWindow, Theme};

fn botao(app: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .unwrap_or_else(|| panic!("elemento com rótulo {label:?} não encontrado"))
}

#[test]
fn escolher_um_tema_aplica_e_grava() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_config(dir.path(), "# meu arquivo\n[font]\nsize = 13\n");
    c.app.borrow_mut().new_tab_with(cmd("tema"));
    let linha = esperar_texto(&c, 0, "tema") as usize;
    assert_eq!(app.get_theme_index(), 0, "sem `theme`, o primeiro");

    assert!(achar(&app, "AppWindow::theme-picker").is_none());
    botao(&app, "Configurações").invoke_accessible_default_action();
    botao(&app, "Temas").invoke_accessible_default_action();
    assert!(achar(&app, "AppWindow::theme-picker").is_some());
    assert!(!app.get_settings_open(), "o menu dá lugar à lista");
    let nomes: Vec<_> = app
        .get_themes()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(nomes.contains(&"Dracula".to_string()), "{nomes:?}");
    assert_eq!(app.get_themes_note(), "");

    botao(&app, "Tema Dracula").invoke_accessible_default_action();
    let dracula = nomes.iter().position(|n| n == "Dracula").unwrap() as i32;
    assert_eq!(app.get_theme_index(), dracula);
    let theme = app.global::<Theme>();
    assert_eq!(
        theme.get_background(),
        slint::Color::from_rgb_u8(0x28, 0x2a, 0x36)
    );
    assert_eq!(
        theme.get_accent(),
        slint::Color::from_rgb_u8(0xbd, 0x93, 0xf9)
    );
    // O painel que já estava aberto é redesenhado com as cores novas.
    let fg = c.app.borrow().panes[&0].shown[linha][0].fg;
    assert_eq!(fg, (0xf8, 0xf8, 0xf2));
    assert!(
        achar(&app, "AppWindow::theme-picker").is_some(),
        "continua aberta para comparar"
    );

    let texto = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(texto.contains("theme = \"Dracula\""), "{texto}");
    assert!(
        texto.contains("# meu arquivo") && texto.contains("size = 13"),
        "{texto}"
    );
    assert_eq!(app.get_config_error(), "");

    // A engrenagem fecha a lista (e abre o menu de novo).
    botao(&app, "Configurações").invoke_accessible_default_action();
    assert!(achar(&app, "AppWindow::theme-picker").is_none());
    assert!(app.get_settings_open());
}

#[test]
fn tema_do_arquivo_e_aviso_de_cores_por_cima() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _c) = abrir_com_config(
        dir.path(),
        "theme = \"Gruvbox Dark\"\n[app]\naccent = \"#ff0000\"\n",
    );
    let nome = app
        .get_themes()
        .row_data(app.get_theme_index() as usize)
        .unwrap()
        .name;
    assert_eq!(nome, "Gruvbox Dark");
    let theme = app.global::<Theme>();
    assert_eq!(
        theme.get_background(),
        slint::Color::from_rgb_u8(0x28, 0x28, 0x28)
    );
    assert_eq!(theme.get_accent(), slint::Color::from_rgb_u8(0xff, 0, 0));
    assert!(app.get_themes_note().contains("[colors] ou [app]"));
}

#[test]
#[ignore]
fn captura() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_config(dir.path(), "theme = \"Tokyo Night\"");
    c.app
        .borrow_mut()
        .new_tab_with(cmd("Nemux com Tokyo Night"));
    esperar_texto(&c, 0, "Nemux");
    app.invoke_toggle_themes();
    println!("{}", capturar(&app, "ThemePicker::card", "temas").display());
    println!(
        "{}",
        capturar(&app, "AppWindow::body", "temas-janela").display()
    );
}
