// Configuração do usuário (config.toml): tema, fonte, cores do terminal e aviso de erro.

use std::path::Path;

use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;

use super::{abrir, achar, capturar, cmd, esperar, esperar_texto};
use crate::{AppWindow, Controllers, Theme};

/// Liga os controllers de `app` lendo a configuração `texto` (num arquivo temporário).
fn ligar_com_config(app: &AppWindow, dir: &Path, texto: &str) -> Controllers {
    let config = dir.join("config.toml");
    std::fs::write(&config, texto).unwrap();
    crate::init_controllers_with(app, dir.join("memoria.db"), Some(&config))
}

fn abrir_com_config(dir: &Path, texto: &str) -> (AppWindow, Controllers) {
    let app = abrir();
    let c = ligar_com_config(&app, dir, texto);
    (app, c)
}

const TEMA: &str = r##"
[font]
size = 20

[colors]
foreground = "#e0def4"
background = "#191724"
palette = ["#26233a", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4",
           "#6e6a86", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4"]

[app]
accent = "#c4a7e7"
tab_bar = "#1f1d2e"
"##;

#[test]
fn aplica_fonte_e_cores() {
    let dir = tempfile::tempdir().unwrap();
    let app = abrir();
    let altura_padrao = app.get_cell_height();
    let c = ligar_com_config(&app, dir.path(), TEMA);
    let theme = app.global::<Theme>();
    assert_eq!(theme.get_font_size(), 20.0);
    assert_eq!(
        theme.get_background(),
        slint::Color::from_rgb_u8(0x19, 0x17, 0x24)
    );
    assert_eq!(theme.get_tab_active(), theme.get_background());
    assert_eq!(
        theme.get_accent(),
        slint::Color::from_rgb_u8(0xc4, 0xa7, 0xe7)
    );
    assert!(
        app.get_cell_height() > altura_padrao,
        "fonte maior, célula maior"
    );
    assert_eq!(app.get_config_error(), "");

    // O terminal desenha com as cores configuradas.
    c.app.borrow_mut().new_tab_with(cmd("tema"));
    let linha = esperar_texto(&c, 0, "tema");
    let estado = c.app.borrow();
    let run = &estado.panes[&0].shown[linha as usize][0];
    assert_eq!(run.fg, (0xe0, 0xde, 0xf4));
    assert_eq!(estado.colors.palette[1], (0xeb, 0x6f, 0x92));
}

#[test]
fn shell_da_configuracao() {
    if std::env::var_os("NEMUX_SHELL").is_some() {
        return; // a variável tem prioridade sobre a configuração
    }
    let dir = tempfile::tempdir().unwrap();
    let (_app, c) = abrir_com_config(dir.path(), "shell = \"cmd.exe /k echo do-config\"");
    let cmd = (c.app.borrow().shell)();
    let argv: Vec<_> = cmd.get_argv().iter().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(argv, ["cmd.exe", "/k", "echo", "do-config"]);
}

#[test]
fn erro_mostra_aviso_e_usa_o_padrao() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_config(dir.path(), "[colors]\nbackground = \"preto\"");
    assert!(
        app.get_config_error().contains("cor inválida \"preto\""),
        "{}",
        app.get_config_error()
    );
    assert!(achar(&app, "AppWindow::config-banner").is_some());
    assert_eq!(c.app.borrow().colors, crate::terminal::Colors::default());

    // A faixa ocupa espaço: fechar devolve a altura aos painéis.
    c.app.borrow_mut().new_tab_with(cmd(""));
    esperar(&c, "o painel", |e| !e.panes.is_empty());
    let altura = app.get_area_height();
    ElementHandle::find_by_accessible_label(&app, "Fechar aviso da configuração")
        .next()
        .expect("botão de fechar o aviso")
        .invoke_accessible_default_action();
    assert_eq!(app.get_config_error(), "");
    assert!(achar(&app, "AppWindow::config-banner").is_none());
    assert_eq!(app.get_area_height(), altura + 28.0);
}

#[test]
#[ignore]
fn captura_tema() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_config(dir.path(), TEMA);
    c.app.borrow_mut().new_tab_with(cmd("Nemux com tema"));
    esperar_texto(&c, 0, "Nemux");
    // Uma segunda aba, para ver a ativa e a inativa; volta para a primeira.
    c.app.borrow_mut().new_tab_with(cmd(""));
    c.app.borrow_mut().select_tab(0);
    c.app.borrow_mut().render(&app);
    println!(
        "{}",
        capturar(&app, "AppWindow::tab-bar", "config-tema-abas").display()
    );
    println!(
        "{}",
        capturar(&app, "AppWindow::body", "config-tema").display()
    );
}

#[test]
#[ignore]
fn captura_erro() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_config(dir.path(), "[font]\nsize = 3");
    c.app.borrow_mut().new_tab_with(cmd(""));
    esperar(&c, "o painel", |e| !e.panes.is_empty());
    println!(
        "{}",
        capturar(&app, "AppWindow::config-banner", "config-erro").display()
    );
}
