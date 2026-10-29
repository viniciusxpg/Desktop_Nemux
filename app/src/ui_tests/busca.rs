// Busca no histórico (Ctrl+Shift+F): ocorrências destacadas, Enter/Shift+Enter navegam.

use i_slint_backend_testing::ElementHandle;
use portable_pty::CommandBuilder;
use slint::platform::{Key, WindowEvent};
use slint::{Model, SharedString};

use super::{abrir_com_cmd, capturar, elemento, enviar, esperar, esperar_texto, painel};
use crate::{AppWindow, Controllers};

/// Um `cmd.exe` com 80 linhas numeradas: a maior parte vai para o histórico.
fn linhas() -> CommandBuilder {
    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.args(["/k", "for /L %i in (1,1,80) do @echo linha %i"]);
    cmd
}

fn cenario() -> (AppWindow, Controllers) {
    let (app, c) = abrir_com_cmd("");
    {
        let mut estado = c.app.borrow_mut();
        estado.close_tab(0);
        estado.new_tab_with(linhas());
    }
    esperar_texto(&c, 1, "linha 80");
    (app, c)
}

fn tecla(app: &AppWindow, texto: SharedString) {
    enviar(
        app,
        WindowEvent::KeyPressed {
            text: texto.clone(),
        },
    );
    enviar(app, WindowEvent::KeyReleased { text: texto });
}

fn ctrl_shift_f(app: &AppWindow) {
    for k in [Key::Control, Key::Shift] {
        enviar(app, WindowEvent::KeyPressed { text: k.into() });
    }
    tecla(app, "F".into());
    for k in [Key::Shift, Key::Control] {
        enviar(app, WindowEvent::KeyReleased { text: k.into() });
    }
}

fn digitar_busca(app: &AppWindow, texto: &str) {
    elemento(app, "SearchBar::input").set_accessible_value(texto);
}

/// Texto da linha da tela onde está a ocorrência escolhida.
fn linha_escolhida(app: &AppWindow, c: &Controllers) -> String {
    let p = painel(app, 1);
    assert!(
        p.current_match.row >= 0,
        "a ocorrência escolhida não está à vista"
    );
    // `shown` é atualizado pelo render; o painel acabou de ser desenhado.
    let estado = c.app.borrow();
    estado.panes[&1].shown[p.current_match.row as usize]
        .iter()
        .map(|r| r.text.as_str())
        .collect()
}

#[test]
fn acha_no_historico_e_navega() {
    let (app, c) = cenario();
    app.invoke_focus_terminal();
    ctrl_shift_f(&app);
    assert!(app.get_search_open());

    digitar_busca(&app, "LINHA 7");
    // "linha 7" e "linha 70".."linha 79"; começa pela mais perto do fim.
    assert_eq!(app.get_search_status(), "11 de 11");
    assert_eq!(linha_escolhida(&app, &c), "linha 79");
    assert!(painel(&app, 1).matches.row_count() >= 10);

    // Enter sobe; Shift+Enter desce e dá a volta.
    tecla(&app, Key::Return.into());
    assert_eq!(app.get_search_status(), "10 de 11");
    assert_eq!(linha_escolhida(&app, &c), "linha 78");
    enviar(
        &app,
        WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        },
    );
    tecla(&app, Key::Return.into());
    tecla(&app, Key::Return.into());
    enviar(
        &app,
        WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        },
    );
    assert_eq!(app.get_search_status(), "1 de 11");
    assert!(painel(&app, 1).offset > 0, "rolou até o histórico");
    assert_eq!(linha_escolhida(&app, &c), "linha 7");

    // O botão "acima" dá a volta para a última.
    ElementHandle::find_by_accessible_label(&app, "Ocorrência acima (Enter)")
        .next()
        .unwrap()
        .invoke_accessible_default_action();
    assert_eq!(app.get_search_status(), "11 de 11");

    digitar_busca(&app, "não tem");
    assert_eq!(app.get_search_status(), "Nenhum resultado");
    assert_eq!(painel(&app, 1).current_match.row, -1);
    assert_eq!(painel(&app, 1).matches.row_count(), 0);
}

#[test]
fn esc_fecha_e_reabrir_busca_de_novo() {
    let (app, c) = cenario();
    app.invoke_focus_terminal();
    ctrl_shift_f(&app);
    digitar_busca(&app, "linha 3");
    assert_eq!(app.get_search_status(), "11 de 11");

    tecla(&app, Key::Escape.into());
    assert!(!app.get_search_open());
    assert!(c.app.borrow().search.is_none());
    assert_eq!(painel(&app, 1).matches.row_count(), 0);

    // Reabrir mantém o texto e volta a destacar.
    ctrl_shift_f(&app);
    assert_eq!(app.get_search_query(), "linha 3");
    assert_eq!(app.get_search_status(), "11 de 11");
    assert_eq!(linha_escolhida(&app, &c), "linha 39");
}

#[test]
fn com_a_busca_aberta_as_teclas_nao_vao_para_o_terminal() {
    let (app, c) = cenario();
    app.invoke_focus_terminal();
    ctrl_shift_f(&app);
    tecla(&app, "x".into());
    tecla(&app, "y".into());
    assert_eq!(app.get_search_query(), "xy");
    std::thread::sleep(std::time::Duration::from_millis(500));
    esperar(&c, "o terminal desenhar", |_| true);
    let estado = c.app.borrow();
    let tela: Vec<String> = estado.panes[&1]
        .shown
        .iter()
        .map(|l| l.iter().map(|r| r.text.as_str()).collect())
        .collect();
    assert!(
        !tela.iter().any(|l| l.contains("xy")),
        "o terminal recebeu as teclas: {tela:?}"
    );
}

#[test]
#[ignore]
fn captura() {
    let (app, c) = cenario();
    app.invoke_focus_terminal();
    ctrl_shift_f(&app);
    digitar_busca(&app, "linha 7");
    tecla(&app, Key::Return.into());
    c.app.borrow_mut().render(&app);
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "busca").display()
    );
}
