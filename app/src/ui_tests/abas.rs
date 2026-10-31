// Abas: renomear (duplo clique) e reordenar (arrastar ou Ctrl+Shift+PageUp/PageDown).

use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{LogicalPosition, Model, SharedString};

use super::{LARGURA, abrir, abrir_com_cmd, capturar, cmd, elemento, enviar, esperar};
use crate::terminal::Event;
use crate::{AppWindow, Controllers};

/// Três abas com títulos fixos ("um", "dois", "três"), a última ativa.
fn tres_abas() -> (AppWindow, Controllers) {
    let (app, c) = abrir_com_cmd("");
    {
        let mut estado = c.app.borrow_mut();
        estado.new_tab_with(cmd(""));
        estado.new_tab_with(cmd(""));
    }
    esperar(&c, "três abas", |e| e.tabs.len() == 3);
    {
        let mut estado = c.app.borrow_mut();
        for (pane, titulo) in [(0, "um"), (1, "dois"), (2, "três")] {
            estado.on_event(pane, &Event::Title(titulo.into()));
        }
        estado.render(&app);
    }
    (app, c)
}

fn titulos(app: &AppWindow) -> Vec<String> {
    app.get_tabs().iter().map(|t| t.title.to_string()).collect()
}

/// Centro da aba `i` na janela.
fn centro(app: &AppWindow, i: usize) -> LogicalPosition {
    let aba = ElementHandle::find_by_element_id(app, "TabButton::touch")
        .nth(i)
        .expect("aba");
    let (p, s) = (aba.absolute_position(), aba.size());
    LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
}

fn apertar(app: &AppWindow, position: LogicalPosition) {
    let button = PointerEventButton::Left;
    enviar(app, WindowEvent::PointerMoved { position });
    enviar(app, WindowEvent::PointerPressed { position, button });
}

fn soltar(app: &AppWindow, position: LogicalPosition) {
    let button = PointerEventButton::Left;
    enviar(app, WindowEvent::PointerReleased { position, button });
}

fn duplo_clique(app: &AppWindow, i: usize) {
    let p = centro(app, i);
    for _ in 0..2 {
        apertar(app, p);
        soltar(app, p);
    }
}

/// O campo de nome de alguma aba está aberto.
fn editando(app: &AppWindow) -> bool {
    ElementHandle::find_by_accessible_label(app, "Nome da aba")
        .next()
        .is_some()
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

fn digitar(app: &AppWindow, texto: &str) {
    for c in texto.chars() {
        tecla(app, c.to_string().into());
    }
}

#[test]
fn duplo_clique_renomeia_e_o_nome_fica() {
    let (app, c) = tres_abas();
    duplo_clique(&app, 0);
    assert_eq!(app.get_active_tab(), 0, "o clique também escolhe a aba");
    assert!(editando(&app));

    // O texto vem todo selecionado: digitar troca o nome inteiro.
    digitar(&app, "API");
    tecla(&app, Key::Return.into());
    assert_eq!(titulos(&app), ["API", "dois", "três"]);
    assert!(!editando(&app));
    assert_eq!(c.app.borrow().tabs[0].name.as_deref(), Some("API"));

    // O programa mudar o título não apaga o nome.
    {
        let mut estado = c.app.borrow_mut();
        estado.on_event(0, &Event::Title("claude".into()));
        estado.render(&app);
    }
    assert_eq!(titulos(&app)[0], "API");

    // Um nome vazio volta ao título do programa.
    duplo_clique(&app, 0);
    tecla(&app, Key::Backspace.into());
    tecla(&app, Key::Return.into());
    assert_eq!(titulos(&app)[0], "claude");
    assert_eq!(c.app.borrow().tabs[0].name, None);
}

#[test]
fn esc_desiste_de_renomear() {
    let (app, c) = tres_abas();
    duplo_clique(&app, 1);
    digitar(&app, "outro");
    tecla(&app, Key::Escape.into());
    assert_eq!(titulos(&app), ["um", "dois", "três"]);
    assert!(!editando(&app));
    assert_eq!(c.app.borrow().tabs[1].name, None);

    // Com o campo fechado, as teclas voltam ao terminal (não ao campo).
    tecla(&app, "x".into());
    assert_eq!(c.app.borrow().tabs[1].name, None);
}

#[test]
fn arrastar_reordena() {
    let (app, c) = tres_abas();
    // A primeira aba vai para o lugar da última.
    let (de, ate) = (centro(&app, 0), centro(&app, 2));
    apertar(&app, de);
    enviar(&app, WindowEvent::PointerMoved { position: ate });
    soltar(&app, ate);
    assert_eq!(titulos(&app), ["dois", "três", "um"]);
    assert_eq!(app.get_active_tab(), 2, "a aba arrastada fica ativa");
    assert_eq!(c.app.borrow().active, 2);

    // Um movimento pequeno é só um clique.
    let p = centro(&app, 1);
    apertar(&app, p);
    enviar(
        &app,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(p.x + 4.0, p.y),
        },
    );
    soltar(&app, p);
    assert_eq!(titulos(&app), ["dois", "três", "um"]);
    assert_eq!(app.get_active_tab(), 1);

    // Arrastar para além da última aba leva para o fim.
    let fora = LogicalPosition::new(centro(&app, 2).x + 2000.0, p.y);
    apertar(&app, centro(&app, 0));
    enviar(&app, WindowEvent::PointerMoved { position: fora });
    soltar(&app, fora);
    assert_eq!(titulos(&app), ["três", "um", "dois"]);
}

#[test]
fn ctrl_shift_page_move_a_aba_ativa() {
    let (app, c) = tres_abas();
    app.invoke_focus_terminal();
    let ctrl_shift = |tecla_: Key| {
        for k in [Key::Control, Key::Shift] {
            enviar(&app, WindowEvent::KeyPressed { text: k.into() });
        }
        tecla(&app, tecla_.into());
        for k in [Key::Shift, Key::Control] {
            enviar(&app, WindowEvent::KeyReleased { text: k.into() });
        }
    };
    // A ativa é a "três", a última.
    ctrl_shift(Key::PageUp);
    assert_eq!(titulos(&app), ["um", "três", "dois"]);
    ctrl_shift(Key::PageUp);
    ctrl_shift(Key::PageUp);
    assert_eq!(titulos(&app), ["três", "um", "dois"], "para na ponta");
    ctrl_shift(Key::PageDown);
    assert_eq!(titulos(&app), ["um", "três", "dois"]);
    assert_eq!(c.app.borrow().active, 1);
}

#[test]
fn mover_mantem_a_aba_ativa() {
    let (_app, c) = tres_abas();
    let mut estado = c.app.borrow_mut();
    estado.select_tab(1);
    estado.move_tab(0, 2);
    assert_eq!(estado.active, 0, "a ativa andou para a esquerda");
    estado.move_tab(2, 0);
    assert_eq!(estado.active, 1);
    estado.move_tab(1, 1);
    estado.move_tab(7, 0);
    assert_eq!(estado.active, 1);
}

#[test]
#[ignore]
fn captura() {
    let (app, _c) = tres_abas();
    // Arrastando a primeira sobre a terceira (sem soltar) e renomeando a segunda.
    apertar(&app, centro(&app, 0));
    enviar(
        &app,
        WindowEvent::PointerMoved {
            position: centro(&app, 2),
        },
    );
    println!(
        "{}",
        capturar(&app, "AppWindow::tab-bar", "abas-arrastando").display()
    );
    soltar(&app, centro(&app, 0));
    duplo_clique(&app, 1);
    digitar(&app, "API");
    println!(
        "{}",
        capturar(&app, "AppWindow::tab-bar", "abas-renomeando").display()
    );
}

/// Janela com `n` abas ("aba 1" a "aba n") e a aba `ativa`, sem terminais.
fn muitas_abas(n: usize, ativa: usize) -> AppWindow {
    let app = abrir();
    let abas: Vec<crate::TabInfo> = (1..=n)
        .map(|i| crate::TabInfo {
            title: format!("aba {i}").into(),
            ..Default::default()
        })
        .collect();
    app.set_tabs(slint::ModelRc::new(slint::VecModel::from(abas)));
    app.set_active_tab(ativa as i32);
    quadro();
    app
}

/// Deixa o Slint rodar os `changed` (como o laço de eventos do app faria).
fn quadro() {
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(16));
}

/// Começo e fim (x) de um elemento na janela.
fn faixa(e: &ElementHandle) -> (f32, f32) {
    let (p, s) = (e.absolute_position(), e.size());
    (p.x, p.x + s.width)
}

#[test]
fn muitas_abas_nao_empurram_os_botoes_para_fora() {
    let app = muitas_abas(20, 19);
    // Os botões da direita continuam dentro da janela.
    let (_, fim) = faixa(&elemento(&app, "AppWindow::settings-btn"));
    assert!(
        fim <= LARGURA,
        "engrenagem termina em {fim}, fora da janela de {LARGURA}"
    );

    // A aba ativa (a última) fica à vista, dentro da faixa das abas.
    let (ini_faixa, fim_faixa) = faixa(&elemento(&app, "AppWindow::tab-strip"));

    // A busca só acha as abas à vista.
    let aba = ElementHandle::find_by_accessible_label(&app, "aba 20")
        .next()
        .expect("aba 20 à vista");
    let (ini, fim) = faixa(&aba);
    assert!(
        ini >= ini_faixa - 0.5 && fim <= fim_faixa + 0.5,
        "aba ativa em {ini}..{fim}, faixa em {ini_faixa}..{fim_faixa}"
    );

    // Voltar para a primeira a traz de volta para a vista.
    app.set_active_tab(0);
    quadro();
    let aba = ElementHandle::find_by_accessible_label(&app, "aba 1")
        .next()
        .expect("aba 1 à vista");
    let (ini, _) = faixa(&aba);
    assert!(
        ini >= ini_faixa - 0.5,
        "aba 1 começa em {ini}, antes da faixa"
    );
}

#[test]
fn poucas_abas_ficam_no_tamanho_natural() {
    let app = muitas_abas(2, 0);
    let aba = ElementHandle::find_by_element_id(&app, "TabButton::touch")
        .next()
        .expect("aba");
    let (ini, fim) = faixa(&aba);
    assert_eq!(fim - ini, 220.0, "largura máxima com espaço sobrando");
}
