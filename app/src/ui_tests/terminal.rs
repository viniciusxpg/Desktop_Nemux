use std::cell::Cell;
use std::num::NonZeroU16;
use std::rc::Rc;

use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};

use super::{
    ALTURA, LARGURA, abrir, abrir_com_cmd, capturar, elemento, enviar, esperar, medir, painel,
};
use crate::layout::{Rect, grid_size};
use crate::terminal::{Callbacks, Parser, snapshot};
use crate::{AppWindow, PaneView, TabInfo};

/// Passa `ansi` por um parser vt100 do tamanho de um painel que ocupa a área toda
/// e o mostra na janela, do mesmo jeito que `App::render` faz.
fn mostrar(app: &AppWindow, ansi: &str) {
    let (w, h) = (app.get_area_width(), app.get_area_height());
    let (cols, rows) = grid_size(
        Rect::new(0.0, 0.0, w, h),
        app.get_cell_width(),
        app.get_cell_height(),
        app.get_pane_pad(),
        app.get_scrollbar_width(),
    );
    let mut parser = Parser::new_with_callbacks(
        NonZeroU16::new(rows).unwrap(),
        NonZeroU16::new(cols).unwrap(),
        0,
        Callbacks::default(),
    );
    parser.process(ansi.as_bytes());
    let snap = snapshot(parser.screen());

    let lines: Vec<_> = snap.lines.iter().map(|l| crate::app::to_line(l)).collect();
    let (cursor_row, cursor_col) = snap.cursor.map_or((-1, 0), |(r, c)| (r as i32, c as i32));
    let view = PaneView {
        id: 0,
        x: 0.0,
        y: 0.0,
        width: w,
        height: h,
        lines: ModelRc::new(VecModel::from(lines)),
        cursor_row,
        cursor_col,
        selection: ModelRc::default(),
        active: true,
        rows: rows as i32,
        history: 0,
        offset: 0,
    };
    app.set_panes(ModelRc::new(VecModel::from(vec![view])));
}

fn abas(app: &AppWindow, titulos: &[&str], ativa: i32) {
    let tabs: Vec<_> = titulos
        .iter()
        .map(|t| TabInfo { title: (*t).into() })
        .collect();
    app.set_tabs(ModelRc::new(VecModel::from(tabs)));
    app.set_active_tab(ativa);
}

fn aba(app: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .unwrap_or_else(|| panic!("elemento com rótulo {label:?} não encontrado"))
}

#[test]
fn area_dos_paineis_fica_abaixo_das_abas() {
    let app = abrir();
    let (cw, ch) = (app.get_cell_width(), app.get_cell_height());
    assert!(
        cw > 5.0 && ch > 10.0,
        "célula com tamanho estranho: {cw}×{ch}"
    );
    // 32px de barra de abas.
    assert_eq!(
        (app.get_area_width(), app.get_area_height()),
        (LARGURA, ALTURA - 32.0)
    );

    mostrar(&app, "oi");
    let grade = elemento(&app, "TerminalPane::grid");
    let pad = app.get_pane_pad();
    assert_eq!(
        grade.size().width,
        LARGURA - 2.0 * pad - app.get_scrollbar_width(),
        "a barra de rolagem tem faixa própria"
    );
    assert_eq!(grade.size().height, ALTURA - 32.0 - 2.0 * pad);
}

#[test]
fn um_painel_ocupa_a_area_e_o_terminal_cabe_nele() {
    let (app, controllers) = abrir_com_cmd("oi");
    esperar(&controllers, "o painel aparecer", |_| {
        app.get_panes().row_count() == 1
    });

    let p = painel(&app, 0);
    assert_eq!(
        (p.x, p.y, p.width, p.height),
        (0.0, 0.0, LARGURA, ALTURA - 32.0)
    );
    let esperado = grid_size(
        Rect::new(0.0, 0.0, p.width, p.height),
        app.get_cell_width(),
        app.get_cell_height(),
        app.get_pane_pad(),
        app.get_scrollbar_width(),
    );
    let estado = controllers.app.borrow();
    let pane = &estado.panes[&0];
    assert_eq!((pane.cols, pane.rows), esperado);
}

#[test]
fn redimensionar_a_janela_redimensiona_o_terminal() {
    let (app, controllers) = abrir_com_cmd("oi");
    app.window().set_size(slint::LogicalSize::new(600.0, 300.0));
    // Os `changed` rodam no próximo ciclo.
    esperar(&controllers, "o terminal encolher", |e| {
        e.panes[&0].cols < 80
    });

    let p = painel(&app, 0);
    assert_eq!((p.width, p.height), (600.0, 300.0 - 32.0));
    let esperado = grid_size(
        Rect::new(0.0, 0.0, 600.0, 268.0),
        app.get_cell_width(),
        app.get_cell_height(),
        app.get_pane_pad(),
        app.get_scrollbar_width(),
    );
    let estado = controllers.app.borrow();
    assert_eq!((estado.panes[&0].cols, estado.panes[&0].rows), esperado);
}

#[test]
fn mostra_texto_e_cursor() {
    let app = abrir();
    mostrar(&app, "PS C:\\> echo oi\r\noi\r\nPS C:\\> ");
    let p = painel(&app, 0);
    assert_eq!((p.cursor_row, p.cursor_col), (2, 8));

    mostrar(&app, "\x1b[?25l");
    assert_eq!(painel(&app, 0).cursor_row, -1);
}

#[test]
fn clicar_na_aba_seleciona() {
    let app = abrir();
    abas(&app, &["PowerShell", "claude"], 0);
    let escolhida = Rc::new(Cell::new(-1));
    app.on_select_tab({
        let escolhida = escolhida.clone();
        move |i| escolhida.set(i)
    });

    aba(&app, "claude").invoke_accessible_default_action();
    assert_eq!(escolhida.get(), 1);
}

/// Um terminal que redesenha entre o apertar e o soltar do botão não pode engolir o clique.
#[test]
fn clicar_na_aba_com_o_terminal_redesenhando() {
    let (app, c) = abrir_com_cmd("um");
    app.invoke_new_tab();
    esperar(&c, "a segunda aba", |e| e.tabs.len() == 2);
    assert_eq!(app.get_active_tab(), 1);

    let primeira = ElementHandle::find_by_element_id(&app, "TabButton::touch")
        .next()
        .expect("botão da primeira aba");
    let (p, s) = (primeira.absolute_position(), primeira.size());
    let ponto = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    let button = PointerEventButton::Left;
    enviar(&app, WindowEvent::PointerMoved { position: ponto });
    enviar(
        &app,
        WindowEvent::PointerPressed {
            position: ponto,
            button,
        },
    );
    c.app.borrow_mut().render(&app);
    enviar(
        &app,
        WindowEvent::PointerReleased {
            position: ponto,
            button,
        },
    );
    assert_eq!(app.get_active_tab(), 0);
}

#[test]
fn botoes_de_fechar_e_nova_aba() {
    let app = abrir();
    abas(&app, &["um", "dois"], 0);
    let fechada = Rc::new(Cell::new(-1));
    let novas = Rc::new(Cell::new(0));
    app.on_close_tab({
        let fechada = fechada.clone();
        move |i| fechada.set(i)
    });
    app.on_new_tab({
        let novas = novas.clone();
        move || novas.set(novas.get() + 1)
    });

    aba(&app, "Fechar aba dois").invoke_accessible_default_action();
    aba(&app, "Nova aba").invoke_accessible_default_action();
    assert_eq!((fechada.get(), novas.get()), (1, 1));
}

// Não verifica nada: gera as capturas e as medidas para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let app = abrir();
    abas(
        &app,
        &["Windows PowerShell", "claude — nemux", "Terminal 3"],
        1,
    );
    mostrar(
        &app,
        concat!(
            "\x1b[38;5;208m╭──────────────────────────────────────────╮\x1b[0m\r\n",
            "\x1b[38;5;208m│\x1b[0m \x1b[1m✻ Welcome to Claude Code!\x1b[0m                \x1b[38;5;208m│\x1b[0m\r\n",
            "\x1b[38;5;208m│\x1b[0m   \x1b[2mcwd: C:\\Users\\Intel\\nemux\x1b[0m              \x1b[38;5;208m│\x1b[0m\r\n",
            "\x1b[38;5;208m╰──────────────────────────────────────────╯\x1b[0m\r\n",
            "\r\n",
            "\x1b[32m✓\x1b[0m cargo test: \x1b[1;32m15 passed\x1b[0m, \x1b[31m0 failed\x1b[0m\r\n",
            "\x1b[33mwarning\x1b[0m: \x1b[4munused import\x1b[0m em \x1b[3msrc/main.rs\x1b[0m\r\n",
            "\x1b[44;97m INFO \x1b[0m fundo azul  \x1b[7m invertido \x1b[0m  \x1b[38;2;255;105;180mrosa RGB\x1b[0m\r\n",
            "cores: \x1b[30m■\x1b[31m■\x1b[32m■\x1b[33m■\x1b[34m■\x1b[35m■\x1b[36m■\x1b[37m■",
            "\x1b[90m■\x1b[91m■\x1b[92m■\x1b[93m■\x1b[94m■\x1b[95m■\x1b[96m■\x1b[97m■\x1b[0m\r\n",
            "largos: a界b → acentuação: ção ü ñ\r\n",
            "\r\n",
            "\x1b[38;5;244m>\x1b[0m implemente o terminal real"
        ),
    );

    println!("{}", medir(&app, "AppWindow::tab-bar"));
    println!("{}", medir(&app, "AppWindow::term-area"));
    println!("{}", medir(&app, "TerminalPane::grid"));
    println!("célula {}×{}", app.get_cell_width(), app.get_cell_height());
    println!("{}", capturar(&app, "AppWindow::tab-bar", "abas").display());
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "terminal").display()
    );
}
