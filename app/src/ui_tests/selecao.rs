// Seleção com o mouse num terminal de verdade (cmd.exe no ConPTY, sh no Linux). Não testa a
// cópia em si para não mexer na área de transferência de quem roda os testes.

use slint::Model;
use slint::platform::{PointerEventButton, WindowEvent};

use super::{
    abrir_com_cmd, abrir_com_controllers, arrastar, capturar, clicar_em, cmd_linhas, elemento,
    enviar, esperar, esperar_texto, painel, ponto,
};
use crate::{AppWindow, Controllers};

const MARCA: &str = "nemux-selecao";

/// Aba com um painel (id 0) que imprime a marca; devolve a linha dela.
fn abrir_com_marca() -> (AppWindow, Controllers, u16) {
    let (app, controllers) = abrir_com_cmd(&format!("{MARCA} C:\\pasta\\arquivo.rs:42"));
    let linha = esperar_texto(&controllers, 0, MARCA);
    (app, controllers, linha)
}

fn texto_selecionado(controllers: &Controllers) -> Option<String> {
    controllers.app.borrow().selected_text.clone()
}

#[test]
fn arrastar_seleciona_o_texto() {
    let (app, controllers, linha) = abrir_com_marca();

    // De "selecao" até o fim de "nemux-selecao".
    arrastar(&app, ponto(&app, 0, linha, 6), ponto(&app, 0, linha, 12));
    assert_eq!(texto_selecionado(&controllers).as_deref(), Some("selecao"));
    let rects: Vec<_> = painel(&app, 0)
        .selection
        .iter()
        .map(|r| (r.row, r.col, r.width))
        .collect();
    assert_eq!(rects, [(linha as i32, 6, 7)]);

    // Arrastar para trás dá o mesmo resultado.
    arrastar(&app, ponto(&app, 0, linha, 12), ponto(&app, 0, linha, 6));
    assert_eq!(texto_selecionado(&controllers).as_deref(), Some("selecao"));
}

#[test]
fn clique_simples_limpa_a_selecao() {
    let (app, controllers, linha) = abrir_com_marca();
    arrastar(&app, ponto(&app, 0, linha, 0), ponto(&app, 0, linha, 4));
    assert_eq!(texto_selecionado(&controllers).as_deref(), Some("nemux"));

    clicar_em(&app, ponto(&app, 0, linha, 2));
    assert_eq!(texto_selecionado(&controllers), None);
    assert_eq!(painel(&app, 0).selection.row_count(), 0);
}

#[test]
fn duplo_clique_seleciona_caminho() {
    let (app, controllers, linha) = abrir_com_marca();

    let p = ponto(&app, 0, linha, 20);
    let button = PointerEventButton::Left;
    enviar(&app, WindowEvent::PointerMoved { position: p });
    for _ in 0..2 {
        enviar(
            &app,
            WindowEvent::PointerPressed {
                position: p,
                button,
            },
        );
        enviar(
            &app,
            WindowEvent::PointerReleased {
                position: p,
                button,
            },
        );
    }
    assert_eq!(
        texto_selecionado(&controllers).as_deref(),
        Some("C:\\pasta\\arquivo.rs:42")
    );
}

#[test]
fn triplo_clique_seleciona_a_linha() {
    let (app, controllers, linha) = abrir_com_marca();
    let p = ponto(&app, 0, linha, 3);
    for _ in 0..3 {
        clicar_em(&app, p);
    }
    assert_eq!(
        texto_selecionado(&controllers),
        Some(format!("{MARCA} C:\\pasta\\arquivo.rs:42"))
    );
}

#[test]
fn arrastar_para_cima_do_painel_rola_o_historico() {
    let (app, c) = abrir_com_cmd("");
    {
        let mut estado = c.app.borrow_mut();
        estado.close_tab(0);
        estado.new_tab_with(cmd_linhas(80));
    }
    let pane = c.app.borrow().active_pane_id().unwrap();
    let ultima = esperar_texto(&c, pane, "linha 80");
    assert!(painel(&app, pane).history > 0, "precisa de histórico");

    // Aperta na "linha 80" e leva o ponteiro para cima da área dos painéis, parado:
    // o timer do arraste rola o histórico até o topo.
    let de = ponto(&app, pane, ultima, 3);
    let topo = elemento(&app, "AppWindow::term-area").absolute_position().y;
    let acima = slint::LogicalPosition::new(de.x, topo - 20.0);
    let button = PointerEventButton::Left;
    enviar(&app, WindowEvent::PointerMoved { position: de });
    enviar(
        &app,
        WindowEvent::PointerPressed {
            position: de,
            button,
        },
    );
    enviar(&app, WindowEvent::PointerMoved { position: acima });
    esperar(&c, "o histórico rolar até o topo", |_| {
        let p = painel(&app, pane);
        p.offset == p.history
    });
    enviar(
        &app,
        WindowEvent::PointerReleased {
            position: acima,
            button,
        },
    );

    // Da coluna 3 da primeira linha do histórico até a coluna 3 da "linha 80".
    let texto = texto_selecionado(&c).expect("seleção");
    assert!(texto.starts_with("ha 1\nlinha 2\n"), "{texto:?}");
    assert!(texto.ends_with("linha 79\nlinh"), "{texto:?}");
}

#[test]
fn digitar_descarta_a_selecao() {
    let (app, controllers, linha) = abrir_com_marca();
    arrastar(&app, ponto(&app, 0, linha, 0), ponto(&app, 0, linha, 4));
    assert!(texto_selecionado(&controllers).is_some());

    assert!(app.invoke_key_input("x".into(), false, false, false));
    assert_eq!(texto_selecionado(&controllers), None);
}

#[test]
fn clicar_no_terminal_da_foco_ao_teclado() {
    let (app, _controllers, linha) = abrir_com_marca();
    // Registrado depois dos controllers: substitui o callback real.
    let teclas = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    app.on_key_input({
        let teclas = teclas.clone();
        move |texto, _, _, _| {
            teclas.borrow_mut().push_str(&texto);
            true
        }
    });

    clicar_em(&app, ponto(&app, 0, linha + 2, 3));
    enviar(&app, WindowEvent::KeyPressed { text: "x".into() });
    enviar(&app, WindowEvent::KeyReleased { text: "x".into() });
    assert_eq!(teclas.borrow().as_str(), "x");
}

#[test]
fn sem_abas_nada_quebra() {
    let db = std::env::temp_dir().join("nemux-testes-terminal-nao-usado.db");
    let (app, controllers) = abrir_com_controllers(&db);
    app.invoke_select_start(7, 0, 0);
    app.invoke_select_end();
    assert_eq!(texto_selecionado(&controllers), None);
}

// Não verifica nada: gera a captura da seleção para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let (app, controllers, linha) = abrir_com_marca();
    arrastar(
        &app,
        ponto(&app, 0, linha, 6),
        ponto(&app, 0, linha + 1, 10),
    );
    println!("selecionado: {:?}", texto_selecionado(&controllers));
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "selecao").display()
    );
}
