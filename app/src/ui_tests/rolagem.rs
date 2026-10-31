// Rolagem do histórico: roda do mouse e barra de rolagem, com um shell de
// verdade que imprime mais linhas do que cabem na tela.

use slint::LogicalPosition;
use slint::platform::{PointerEventButton, WindowEvent};

use super::{
    abrir_com_cmd, achar, capturar, cmd_linhas, elemento, enviar, esperar, esperar_texto, painel,
    ponto,
};
use crate::{AppWindow, Controllers};

/// Aba com um shell que imprimiu "linha 1" a "linha 80"; espera o histórico encher.
fn abrir_com_historico() -> (AppWindow, Controllers) {
    let (app, c) = abrir_com_cmd("");
    let ps = cmd_linhas(80);
    {
        let mut estado = c.app.borrow_mut();
        estado.close_tab(0);
        estado.new_tab_with(ps);
    }
    let pane = c.app.borrow().active_pane_id().unwrap();
    esperar_texto(&c, pane, "linha 80");
    esperar(&c, "o histórico encher", |_| {
        painel(&app, pane).history > 0
    });
    (app, c)
}

fn pane(c: &Controllers) -> u64 {
    c.app.borrow().active_pane_id().unwrap()
}

fn primeira_linha(c: &Controllers) -> String {
    let estado = c.app.borrow();
    let p = &estado.panes[&estado.active_pane_id().unwrap()];
    p.shown[0].iter().map(|r| r.text.as_str()).collect()
}

fn rodar(app: &AppWindow, onde: LogicalPosition, delta_y: f32) {
    enviar(app, WindowEvent::PointerMoved { position: onde });
    enviar(
        app,
        WindowEvent::PointerScrolled {
            position: onde,
            delta_x: 0.0,
            delta_y,
        },
    );
}

/// Centro de um elemento, em coordenadas da janela.
fn centro(app: &AppWindow, id: &str) -> LogicalPosition {
    let e = elemento(app, id);
    let (p, s) = (e.absolute_position(), e.size());
    LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
}

#[test]
fn sem_historico_nao_tem_barra() {
    let (app, c) = abrir_com_cmd("oi");
    esperar_texto(&c, 0, "oi");
    assert!(achar(&app, "TerminalPane::scrollbar").is_none());
}

#[test]
fn barra_aparece_no_fim_do_historico() {
    let (app, c) = abrir_com_historico();
    let p = painel(&app, pane(&c));
    assert_eq!(p.offset, 0);

    let barra = elemento(&app, "TerminalPane::scrollbar");
    let polegar = elemento(&app, "TerminalPane::thumb");
    let fim_barra = barra.absolute_position().y + barra.size().height;
    let fim_polegar = polegar.absolute_position().y + polegar.size().height;
    assert!(
        (fim_barra - fim_polegar).abs() < 1.0,
        "ao vivo, o polegar fica embaixo"
    );
    assert!(polegar.size().height < barra.size().height);
}

#[test]
fn roda_do_mouse_sobe_e_desce() {
    let (app, c) = abrir_com_historico();
    let id = pane(&c);
    let antes = primeira_linha(&c);
    let onde = ponto(&app, id, 5, 5);

    // Roda para cima: delta positivo, 3 linhas por passo.
    rodar(&app, onde, 60.0);
    assert_eq!(painel(&app, id).offset, 3);
    assert_ne!(primeira_linha(&c), antes);

    rodar(&app, onde, -60.0);
    assert_eq!(painel(&app, id).offset, 0);
    assert_eq!(primeira_linha(&c), antes);
}

#[test]
fn arrastar_o_polegar_ate_o_topo() {
    let (app, c) = abrir_com_historico();
    let id = pane(&c);
    let topo_barra = elemento(&app, "TerminalPane::scrollbar").absolute_position();
    let de = centro(&app, "TerminalPane::thumb");
    let ate = LogicalPosition::new(de.x, topo_barra.y - 50.0);

    let button = PointerEventButton::Left;
    enviar(&app, WindowEvent::PointerMoved { position: de });
    enviar(
        &app,
        WindowEvent::PointerPressed {
            position: de,
            button,
        },
    );
    enviar(&app, WindowEvent::PointerMoved { position: ate });
    enviar(
        &app,
        WindowEvent::PointerReleased {
            position: ate,
            button,
        },
    );

    let p = painel(&app, id);
    assert_eq!(p.offset, p.history, "no topo do histórico");
    assert_eq!(primeira_linha(&c), "linha 1");
    assert_eq!(
        c.app.borrow().selected_text,
        None,
        "arrastar a barra não seleciona texto"
    );
}

#[test]
fn clicar_no_trilho_sobe_uma_pagina() {
    let (app, c) = abrir_com_historico();
    let id = pane(&c);
    let barra = elemento(&app, "TerminalPane::scrollbar").absolute_position();
    let rows = painel(&app, id).rows;

    let acima_do_polegar = LogicalPosition::new(barra.x + 5.0, barra.y + 3.0);
    let button = PointerEventButton::Left;
    enviar(
        &app,
        WindowEvent::PointerMoved {
            position: acima_do_polegar,
        },
    );
    enviar(
        &app,
        WindowEvent::PointerPressed {
            position: acima_do_polegar,
            button,
        },
    );
    enviar(
        &app,
        WindowEvent::PointerReleased {
            position: acima_do_polegar,
            button,
        },
    );

    assert_eq!(painel(&app, id).offset, rows - 1);
}

#[test]
fn digitar_volta_ao_fim() {
    let (app, c) = abrir_com_historico();
    let id = pane(&c);
    rodar(&app, ponto(&app, id, 5, 5), 120.0);
    assert!(painel(&app, id).offset > 0);

    app.invoke_key_input("x".into(), false, false, false);
    assert_eq!(painel(&app, id).offset, 0);
}

// Não verifica nada: gera a captura da barra para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let (app, c) = abrir_com_historico();
    let id = pane(&c);
    rodar(&app, ponto(&app, id, 5, 5), 600.0);
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "rolagem").display()
    );
}
