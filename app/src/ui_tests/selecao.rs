// Seleção com o mouse num terminal de verdade (cmd.exe no ConPTY). Não testa a
// cópia em si para não mexer na área de transferência de quem roda os testes.

use std::time::{Duration, Instant};

use portable_pty::CommandBuilder;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

use super::{abrir_com_controllers, capturar};
use crate::{AppWindow, Controllers};

const MARCA: &str = "nemux-selecao";

/// Abre uma aba com `cmd.exe` que imprime a marca e continua aberto.
fn abrir_com_cmd() -> (AppWindow, Controllers) {
    let (app, controllers) = abrir_com_controllers();
    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.args(["/k", &format!("echo {MARCA} C:\\pasta\\arquivo.rs:42")]);
    controllers.app.borrow_mut().new_tab_with(cmd);
    (app, controllers)
}

/// Roda o timer dos controllers até a marca aparecer na tela; devolve a linha dela.
fn esperar_marca(controllers: &Controllers) -> u16 {
    let limite = Instant::now() + Duration::from_secs(15);
    while Instant::now() < limite {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(20));
        let linha = controllers.app.borrow().shown.iter().position(|runs| {
            let texto: String = runs.iter().map(|r| r.text.as_str()).collect();
            texto.starts_with(MARCA)
        });
        if let Some(linha) = linha {
            return linha as u16;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("a saída do cmd.exe não apareceu");
}

/// Centro da célula, em coordenadas da janela.
fn ponto(app: &AppWindow, linha: u16, coluna: u16) -> LogicalPosition {
    let (cw, ch) = (app.get_cell_width(), app.get_cell_height());
    // 32px de abas + 6px de margem.
    LogicalPosition::new(
        6.0 + (coluna as f32 + 0.5) * cw,
        32.0 + 6.0 + (linha as f32 + 0.5) * ch,
    )
}

fn enviar(app: &AppWindow, evento: WindowEvent) {
    app.window().dispatch_event(evento);
}

fn arrastar(app: &AppWindow, de: LogicalPosition, ate: LogicalPosition) {
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

fn texto_selecionado(controllers: &Controllers) -> Option<String> {
    controllers.app.borrow().selected_text.clone()
}

#[test]
fn arrastar_seleciona_o_texto() {
    let (app, controllers) = abrir_com_cmd();
    let linha = esperar_marca(&controllers);

    // De "selecao" até o fim de "nemux-selecao".
    arrastar(&app, ponto(&app, linha, 6), ponto(&app, linha, 12));
    assert_eq!(texto_selecionado(&controllers).as_deref(), Some("selecao"));
    let rects: Vec<_> = app
        .get_selection()
        .iter()
        .map(|r| (r.row, r.col, r.width))
        .collect();
    assert_eq!(rects, [(linha as i32, 6, 7)]);

    // Arrastar para trás dá o mesmo resultado.
    arrastar(&app, ponto(&app, linha, 12), ponto(&app, linha, 6));
    assert_eq!(texto_selecionado(&controllers).as_deref(), Some("selecao"));
}

#[test]
fn clique_simples_limpa_a_selecao() {
    let (app, controllers) = abrir_com_cmd();
    let linha = esperar_marca(&controllers);
    arrastar(&app, ponto(&app, linha, 0), ponto(&app, linha, 4));
    assert_eq!(texto_selecionado(&controllers).as_deref(), Some("nemux"));

    let p = ponto(&app, linha, 2);
    arrastar(&app, p, p);
    assert_eq!(texto_selecionado(&controllers), None);
    assert_eq!(app.get_selection().row_count(), 0);
}

#[test]
fn duplo_clique_seleciona_caminho() {
    let (app, controllers) = abrir_com_cmd();
    let linha = esperar_marca(&controllers);

    let p = ponto(&app, linha, 20);
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
fn digitar_descarta_a_selecao() {
    let (app, controllers) = abrir_com_cmd();
    let linha = esperar_marca(&controllers);
    arrastar(&app, ponto(&app, linha, 0), ponto(&app, linha, 4));
    assert!(texto_selecionado(&controllers).is_some());

    assert!(app.invoke_key_input("x".into(), false, false, false));
    assert_eq!(texto_selecionado(&controllers), None);
}

#[test]
fn clicar_no_terminal_da_foco_ao_teclado() {
    let (app, _controllers) = abrir_com_controllers();
    let teclas = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    app.on_key_input({
        let teclas = teclas.clone();
        move |texto, _, _, _| {
            teclas.borrow_mut().push_str(&texto);
            true
        }
    });

    let p = ponto(&app, 3, 3);
    arrastar(&app, p, p);
    enviar(&app, WindowEvent::KeyPressed { text: "x".into() });
    enviar(&app, WindowEvent::KeyReleased { text: "x".into() });
    assert_eq!(teclas.borrow().as_str(), "x");
}

// Não verifica nada: gera a captura da seleção para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let (app, controllers) = abrir_com_cmd();
    let linha = esperar_marca(&controllers);
    arrastar(&app, ponto(&app, linha, 6), ponto(&app, linha + 1, 10));
    println!("selecionado: {:?}", texto_selecionado(&controllers));
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "selecao").display()
    );
}
