use std::cell::Cell;
use std::num::NonZeroU16;
use std::rc::Rc;

use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{ALTURA, LARGURA, abrir, abrir_com_controllers, capturar, elemento, medir};
use crate::terminal::{Callbacks, Parser, snapshot};
use crate::{AppWindow, TabInfo};

/// Passa `ansi` por um parser vt100 do tamanho da grade e mostra o resultado na janela,
/// do mesmo jeito que `App::render` faz.
fn mostrar(app: &AppWindow, ansi: &str) {
    let rows = NonZeroU16::new(app.get_grid_rows() as u16).unwrap();
    let cols = NonZeroU16::new(app.get_grid_cols() as u16).unwrap();
    let mut parser = Parser::new_with_callbacks(rows, cols, 0, Callbacks::default());
    parser.process(ansi.as_bytes());
    let snap = snapshot(parser.screen());

    let lines: Vec<_> = snap.lines.iter().map(|l| crate::to_line(l)).collect();
    app.set_lines(ModelRc::new(VecModel::from(lines)));
    let (row, col) = snap.cursor.map_or((-1, 0), |(r, c)| (r as i32, c as i32));
    app.set_cursor_row(row);
    app.set_cursor_col(col);
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
fn grade_preenche_a_area_do_terminal() {
    let app = abrir();
    let (cw, ch) = (app.get_cell_width(), app.get_cell_height());
    assert!(
        cw > 5.0 && ch > 10.0,
        "célula com tamanho estranho: {cw}×{ch}"
    );

    // 32px de barra de abas e 6px de margem em volta da grade.
    let cols = ((LARGURA - 12.0) / cw).floor() as i32;
    let rows = ((ALTURA - 32.0 - 12.0) / ch).floor() as i32;
    assert_eq!((app.get_grid_cols(), app.get_grid_rows()), (cols, rows));

    let grade = elemento(&app, "AppWindow::grid");
    let area = elemento(&app, "AppWindow::term-area");
    assert!(grade.size().width <= area.size().width);
    assert!(grade.size().height <= area.size().height);
}

#[test]
fn redimensionar_a_janela_redimensiona_o_terminal() {
    let (app, controllers) = abrir_com_controllers();
    app.window().set_size(slint::LogicalSize::new(600.0, 300.0));
    // O `changed` roda no próximo ciclo; ler a propriedade força o cálculo.
    let (cols, rows) = (app.get_grid_cols(), app.get_grid_rows());
    processar_pendentes(&app);

    let estado = controllers.app.borrow();
    assert_eq!((estado.cols as i32, estado.rows as i32), (cols, rows));
    assert!(
        cols < 80,
        "600px deveriam caber menos de 80 colunas, deu {cols}"
    );
}

/// Avança o relógio simulado e redesenha, para rodar os `changed` pendentes.
fn processar_pendentes(app: &AppWindow) {
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(16));
    let _ = app.window().take_snapshot();
}

#[test]
fn mostra_texto_e_cursor() {
    let app = abrir();
    mostrar(&app, "PS C:\\> echo oi\r\noi\r\nPS C:\\> ");
    assert_eq!(app.get_cursor_row(), 2);
    assert_eq!(app.get_cursor_col(), 8);

    mostrar(&app, "\x1b[?25l");
    assert_eq!(app.get_cursor_row(), -1);
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
    println!("{}", medir(&app, "AppWindow::grid"));
    println!(
        "célula {}×{}  grade {}×{}",
        app.get_cell_width(),
        app.get_cell_height(),
        app.get_grid_cols(),
        app.get_grid_rows()
    );
    println!("{}", capturar(&app, "AppWindow::tab-bar", "abas").display());
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "terminal").display()
    );
}
