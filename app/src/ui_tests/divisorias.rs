// Arrastar as divisórias entre painéis, com terminais de verdade (cmd.exe no ConPTY).

use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{LogicalPosition, Model};

use super::{LARGURA, abrir_com_cmd, capturar, elemento, enviar, esperar, painel};
use crate::layout::{Rect, grid_size};
use crate::{AppWindow, Controllers};

/// Aba com dois painéis lado a lado (0 à esquerda, 1 à direita).
fn lado_a_lado() -> (AppWindow, Controllers) {
    let (app, c) = abrir_com_cmd("um");
    esperar(&c, "o primeiro painel", |_| {
        app.get_panes().row_count() == 1
    });
    app.invoke_split_right();
    (app, c)
}

/// Faixa de arraste da divisória `n` (na ordem das divisórias da aba).
fn faixa(app: &AppWindow, n: usize) -> ElementHandle {
    ElementHandle::find_by_element_id(app, "AppWindow::grip")
        .nth(n)
        .unwrap_or_else(|| panic!("divisória {n} não encontrada"))
}

fn centro(e: &ElementHandle) -> LogicalPosition {
    let (p, s) = (e.absolute_position(), e.size());
    LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
}

/// Ponto da área dos painéis em coordenadas da janela.
fn na_area(app: &AppWindow, x: f32, y: f32) -> LogicalPosition {
    let area = elemento(app, "AppWindow::term-area").absolute_position();
    LogicalPosition::new(area.x + x, area.y + y)
}

fn arrastar_divisoria(app: &AppWindow, n: usize, ate: LogicalPosition) {
    let de = centro(&faixa(app, n));
    let button = PointerEventButton::Left;
    enviar(app, WindowEvent::PointerMoved { position: de });
    enviar(
        app,
        WindowEvent::PointerPressed {
            position: de,
            button,
        },
    );
    enviar(app, WindowEvent::PointerMoved { position: ate });
    enviar(
        app,
        WindowEvent::PointerReleased {
            position: ate,
            button,
        },
    );
}

fn duplo_clique(app: &AppWindow, onde: LogicalPosition) {
    let button = PointerEventButton::Left;
    enviar(app, WindowEvent::PointerMoved { position: onde });
    for _ in 0..2 {
        enviar(
            app,
            WindowEvent::PointerPressed {
                position: onde,
                button,
            },
        );
        enviar(
            app,
            WindowEvent::PointerReleased {
                position: onde,
                button,
            },
        );
    }
}

/// Colunas e linhas que o terminal do painel deveria ter para o tamanho dele na tela.
fn grade_esperada(app: &AppWindow, pane: u64) -> (u16, u16) {
    let p = painel(app, pane);
    grid_size(
        Rect::new(p.x, p.y, p.width, p.height),
        app.get_cell_width(),
        app.get_cell_height(),
        app.get_pane_pad(),
        app.get_scrollbar_width(),
    )
}

#[test]
fn arrastar_a_divisoria_redimensiona_os_paineis() {
    let (app, c) = lado_a_lado();
    assert_eq!(app.get_dividers().row_count(), 1);

    arrastar_divisoria(&app, 0, na_area(&app, 300.0, 100.0));
    let (a, b) = (painel(&app, 0), painel(&app, 1));
    assert!(
        (a.width - 299.0).abs() <= 1.0,
        "esquerda com ~299px, deu {}",
        a.width
    );
    assert_eq!(a.width + 2.0 + b.width, LARGURA);

    // Os terminais acompanham o novo tamanho.
    let estado = c.app.borrow();
    assert_eq!(
        (estado.panes[&0].cols, estado.panes[&0].rows),
        grade_esperada(&app, 0)
    );
    assert_eq!(
        (estado.panes[&1].cols, estado.panes[&1].rows),
        grade_esperada(&app, 1)
    );
    assert_eq!(
        estado.selected_text, None,
        "arrastar a divisória não seleciona texto"
    );
}

#[test]
fn nenhum_lado_fica_menor_que_o_minimo() {
    let (app, c) = lado_a_lado();

    arrastar_divisoria(&app, 0, na_area(&app, 0.0, 100.0));
    assert!(c.app.borrow().panes[&0].cols >= 20, "esquerda no mínimo");

    arrastar_divisoria(&app, 0, na_area(&app, LARGURA, 100.0));
    assert!(c.app.borrow().panes[&1].cols >= 20, "direita no mínimo");
    assert!(painel(&app, 0).width > LARGURA / 2.0);
}

#[test]
fn duplo_clique_volta_ao_meio() {
    let (app, _c) = lado_a_lado();
    let meio = painel(&app, 0).width;
    arrastar_divisoria(&app, 0, na_area(&app, 250.0, 100.0));
    assert!(painel(&app, 0).width < meio);

    duplo_clique(&app, centro(&faixa(&app, 0)));
    assert_eq!(painel(&app, 0).width, meio);
}

#[test]
fn divisoria_horizontal() {
    let (app, c) = abrir_com_cmd("um");
    esperar(&c, "o primeiro painel", |_| {
        app.get_panes().row_count() == 1
    });
    app.invoke_split_down();
    assert!(!app.get_dividers().row_data(0).unwrap().vertical);

    arrastar_divisoria(&app, 0, na_area(&app, 100.0, 120.0));
    let (cima, baixo) = (painel(&app, 0), painel(&app, 1));
    assert!(
        (cima.height - 119.0).abs() <= 1.0,
        "em cima ~119px, deu {}",
        cima.height
    );
    assert_eq!(baixo.y, cima.height + 2.0);
    assert_eq!(c.app.borrow().panes[&0].rows, grade_esperada(&app, 0).1);
}

#[test]
fn divisoria_interna_so_mexe_nos_seus_paineis() {
    // [0 | 1]
    // [0 | 2]
    let (app, _c) = lado_a_lado();
    app.invoke_split_down();
    assert_eq!(app.get_dividers().row_count(), 2);
    let esquerda = painel(&app, 0).width;

    arrastar_divisoria(&app, 1, na_area(&app, 600.0, 100.0));
    assert_eq!(
        painel(&app, 0).width,
        esquerda,
        "o painel da esquerda não muda"
    );
    assert!(painel(&app, 1).height < painel(&app, 2).height);
}

// Não verifica nada: gera a captura com a divisória deslocada e destacada.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let (app, _c) = lado_a_lado();
    app.invoke_split_down();
    arrastar_divisoria(&app, 0, na_area(&app, 320.0, 100.0));
    // O ponteiro fica sobre a divisória interna, que aparece destacada.
    enviar(
        &app,
        WindowEvent::PointerMoved {
            position: centro(&faixa(&app, 1)),
        },
    );
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "divisorias").display()
    );
}
