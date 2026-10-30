// Menu da engrenagem: fonte (também Ctrl+= / Ctrl+- / Ctrl+0), "restaurar abas",
// abrir o config.toml e a lista de atalhos.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};

use super::config::abrir_com_config;
use super::{abrir, achar, capturar, cmd, enviar, esperar, esperar_texto};
use crate::{AppWindow, Controllers, Theme};

fn botao(app: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .unwrap_or_else(|| panic!("elemento com rótulo {label:?} não encontrado"))
}

fn engrenagem(app: &AppWindow) {
    botao(app, "Configurações").invoke_accessible_default_action();
}

fn arquivo(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default()
}

fn ctrl(app: &AppWindow, texto: &str) {
    enviar(
        app,
        WindowEvent::KeyPressed {
            text: Key::Control.into(),
        },
    );
    enviar(app, WindowEvent::KeyPressed { text: texto.into() });
    enviar(app, WindowEvent::KeyReleased { text: texto.into() });
    enviar(
        app,
        WindowEvent::KeyReleased {
            text: Key::Control.into(),
        },
    );
}

#[test]
fn engrenagem_abre_e_fecha_o_menu() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _c) = abrir_com_config(dir.path(), "");
    assert!(achar(&app, "AppWindow::settings-menu").is_none());
    assert!(
        ElementHandle::find_by_accessible_label(&app, "Temas")
            .next()
            .is_none(),
        "Temas saiu da barra"
    );

    engrenagem(&app);
    assert!(achar(&app, "AppWindow::settings-menu").is_some());
    for item in [
        "Temas",
        "Diminuir a fonte (Ctrl+-)",
        "Aumentar a fonte (Ctrl+=)",
        "Restaurar abas ao abrir",
        "Abrir config.toml",
        "Atalhos de teclado",
    ] {
        botao(&app, item);
    }
    engrenagem(&app);
    assert!(achar(&app, "AppWindow::settings-menu").is_none());

    // Atalhos abre a própria lista, no lugar do menu; Esc fecha.
    engrenagem(&app);
    botao(&app, "Atalhos de teclado").invoke_accessible_default_action();
    assert!(!app.get_settings_open());
    assert!(achar(&app, "AppWindow::shortcuts-card").is_some());
    enviar(
        &app,
        WindowEvent::KeyPressed {
            text: Key::Escape.into(),
        },
    );
    enviar(
        &app,
        WindowEvent::KeyReleased {
            text: Key::Escape.into(),
        },
    );
    assert!(achar(&app, "AppWindow::shortcuts-card").is_none());
}

#[test]
fn fonte_pelo_menu_e_pelo_teclado() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_config(dir.path(), "# meu arquivo\n");
    c.app.borrow_mut().new_tab_with(cmd("fonte"));
    esperar_texto(&c, 0, "fonte");
    let theme = app.global::<Theme>();
    assert_eq!(theme.get_font_size(), 14.0);
    let colunas = c.app.borrow().panes[&0].cols;

    engrenagem(&app);
    botao(&app, "Aumentar a fonte (Ctrl+=)").invoke_accessible_default_action();
    assert_eq!(theme.get_font_size(), 15.0);
    assert!(
        arquivo(dir.path()).contains("size = 15"),
        "{}",
        arquivo(dir.path())
    );
    assert!(arquivo(dir.path()).contains("# meu arquivo"));
    assert!(
        app.get_settings_open(),
        "o menu fica aberto para mais cliques"
    );
    esperar(&c, "o terminal encolher", |e| e.panes[&0].cols < colunas);

    // Pelo teclado, com o foco no terminal.
    engrenagem(&app);
    app.invoke_focus_terminal();
    ctrl(&app, "=");
    ctrl(&app, "=");
    assert_eq!(theme.get_font_size(), 17.0);
    ctrl(&app, "-");
    assert_eq!(theme.get_font_size(), 16.0);
    ctrl(&app, "0");
    assert_eq!(theme.get_font_size(), 14.0);
    assert!(
        !arquivo(dir.path()).contains("size"),
        "{}",
        arquivo(dir.path())
    );
    esperar(&c, "o terminal voltar", |e| e.panes[&0].cols == colunas);

    // Os limites.
    for _ in 0..20 {
        ctrl(&app, "-");
    }
    assert_eq!(theme.get_font_size(), 6.0);
    assert_eq!(app.get_config_error(), "");
}

#[test]
fn restaurar_abas_liga_e_desliga() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let sessao = dir.path().join("session.json");
    let app = abrir();
    let c: Controllers = crate::init_controllers_with(
        &app,
        dir.path().join("memoria.db"),
        Some(&config),
        Some(&sessao),
    );
    c.app.borrow_mut().new_tab_with(cmd("abas"));
    assert!(app.get_restore_tabs());

    engrenagem(&app);
    let item = botao(&app, "Restaurar abas ao abrir");
    assert_eq!(item.accessible_checked(), Some(true));
    item.invoke_accessible_default_action();
    assert!(!app.get_restore_tabs());
    assert!(arquivo(dir.path()).contains("restore = false"));
    c.save_workspace();
    assert!(!sessao.exists(), "desligado, não grava as abas");

    item.invoke_accessible_default_action();
    assert!(arquivo(dir.path()).contains("restore = true"));
    c.save_workspace();
    assert!(sessao.exists());
}

#[test]
fn abrir_config_cria_o_arquivo_e_abre() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("nemux").join("config.toml");
    let app = abrir();
    let c = crate::init_controllers_with(&app, dir.path().join("memoria.db"), Some(&config), None);
    let aberto: Rc<RefCell<Option<PathBuf>>> = Rc::default();
    *c.opener.borrow_mut() = Box::new({
        let aberto = aberto.clone();
        move |p| *aberto.borrow_mut() = Some(p.to_path_buf())
    });

    engrenagem(&app);
    botao(&app, "Abrir config.toml").invoke_accessible_default_action();
    assert_eq!(aberto.borrow().as_deref(), Some(config.as_path()));
    assert!(!app.get_settings_open());
    let texto = std::fs::read_to_string(&config).unwrap();
    assert!(texto.starts_with("# Configuração do Nemux"), "{texto}");
    // O modelo é só comentário: vale como configuração vazia.
    assert_eq!(
        crate::config::Config::parse(&texto).unwrap(),
        crate::config::Config::default()
    );

    // Um arquivo que já existe não é tocado.
    std::fs::write(&config, "theme = \"Dracula\"\n").unwrap();
    engrenagem(&app);
    botao(&app, "Abrir config.toml").invoke_accessible_default_action();
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "theme = \"Dracula\"\n"
    );
}

#[test]
fn sem_config_nao_mostra_abrir() {
    let db = std::env::temp_dir().join("nemux-testes-config-nao-usado.db");
    let (app, _c) = super::abrir_com_controllers(&db);
    engrenagem(&app);
    assert!(
        ElementHandle::find_by_accessible_label(&app, "Abrir config.toml")
            .next()
            .is_none()
    );
}

#[test]
#[ignore]
fn captura() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_config(dir.path(), "");
    c.app.borrow_mut().new_tab_with(cmd("Nemux"));
    esperar_texto(&c, 0, "Nemux");
    println!(
        "{}",
        capturar(&app, "AppWindow::tab-bar", "barra").display()
    );
    app.invoke_toggle_settings();
    println!(
        "{}",
        capturar(&app, "AppWindow::body", "configuracoes").display()
    );
    app.invoke_toggle_shortcuts();
    println!("{}", capturar(&app, "AppWindow::body", "atalhos").display());
}
