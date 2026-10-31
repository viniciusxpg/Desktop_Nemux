// Painéis divididos com terminais de verdade (cmd.exe no ConPTY).

use slint::{ComponentHandle, Model};

use super::{
    ALTURA, LARGURA, PROMPT, abrir_com_cmd, arrastar, capturar, clicar_em, cmd, cmd_que_termina,
    esperar, esperar_texto, painel, ponto,
};
use crate::AppWindow;

fn ids(app: &AppWindow) -> Vec<i32> {
    app.get_panes().iter().map(|p| p.id).collect()
}

fn ativo(app: &AppWindow) -> i32 {
    app.get_panes()
        .iter()
        .find(|p| p.active)
        .expect("algum painel ativo")
        .id
}

/// Digita no painel ativo, como se fosse pelo teclado.
fn digitar(app: &AppWindow, texto: &str) {
    for c in texto.chars() {
        app.invoke_key_input(c.to_string().into(), false, false, false);
    }
}

const ALT: (bool, bool, bool) = (false, true, false);
const ALT_SHIFT: (bool, bool, bool) = (false, true, true);

fn tecla(app: &AppWindow, texto: &str, (ctrl, alt, shift): (bool, bool, bool)) -> bool {
    app.invoke_key_input(texto.into(), ctrl, alt, shift)
}

#[test]
fn dividir_ao_lado_e_abaixo() {
    let (app, c) = abrir_com_cmd("um");
    esperar(&c, "o primeiro painel", |_| {
        app.get_panes().row_count() == 1
    });
    let cols_antes = c.app.borrow().panes[&0].cols;

    app.invoke_split_right();
    assert_eq!(ids(&app), [0, 1]);
    assert_eq!(ativo(&app), 1, "o painel novo recebe o foco");
    let (a, b) = (painel(&app, 0), painel(&app, 1));
    assert_eq!((a.x, a.y, a.height), (0.0, 0.0, ALTURA - 32.0));
    assert_eq!(b.x, a.width + 2.0, "2px de divisória");
    assert_eq!(a.width + 2.0 + b.width, LARGURA);
    {
        let estado = c.app.borrow();
        assert!(estado.panes[&0].cols < cols_antes / 2 + 1);
        assert_eq!(estado.panes[&0].rows, estado.panes[&1].rows);
    }

    app.invoke_split_down();
    assert_eq!(ids(&app), [0, 1, 2]);
    let (b, d) = (painel(&app, 1), painel(&app, 2));
    assert_eq!((d.x, d.width), (b.x, b.width));
    assert_eq!(d.y, b.height + 2.0);
}

#[test]
fn atalhos_dividem_e_movem_o_foco() {
    let (app, c) = abrir_com_cmd("um");
    esperar(&c, "o primeiro painel", |_| {
        app.get_panes().row_count() == 1
    });

    assert!(tecla(&app, "+", ALT_SHIFT));
    assert_eq!((ids(&app), ativo(&app)), (vec![0, 1], 1));
    assert!(tecla(&app, "\u{f702}", ALT)); // Alt+←
    assert_eq!(ativo(&app), 0);
    assert!(tecla(&app, "\u{f703}", ALT)); // Alt+→
    assert_eq!(ativo(&app), 1);

    assert!(tecla(&app, "_", ALT_SHIFT));
    assert_eq!((ids(&app), ativo(&app)), (vec![0, 1, 2], 2));
    assert!(tecla(&app, "\u{f700}", ALT)); // Alt+↑
    assert_eq!(ativo(&app), 1);
    assert!(tecla(&app, "\u{f702}", ALT));
    assert_eq!(ativo(&app), 0);
}

#[test]
fn teclado_vai_para_o_painel_ativo() {
    let (app, c) = abrir_com_cmd("esquerdo");
    esperar_texto(&c, 0, "esquerdo");
    app.invoke_split_right();
    let linha = esperar_texto(&c, 1, PROMPT);

    // Clicar no painel da esquerda o torna ativo.
    clicar_em(&app, ponto(&app, 0, linha, 2));
    assert_eq!(ativo(&app), 0);
    digitar(&app, "echo digitado-aqui\n");
    esperar_texto(&c, 0, "digitado-aqui");

    let estado = c.app.borrow();
    let direita: String = estado.panes[&1]
        .shown
        .iter()
        .flatten()
        .map(|r| r.text.as_str())
        .collect();
    assert!(
        !direita.contains("digitado-aqui"),
        "foi parar no painel errado"
    );
}

#[test]
fn selecionar_no_painel_da_direita() {
    let (app, c) = abrir_com_cmd("um");
    esperar_texto(&c, 0, "um");
    app.invoke_split_right();
    let linha = esperar_texto(&c, 1, PROMPT);

    arrastar(&app, ponto(&app, 1, linha, 0), ponto(&app, 1, linha, 1));
    // As duas primeiras colunas: "C:" no cmd, "$ " no sh.
    let selecionado = c.app.borrow().selected_text.clone();
    assert_eq!(selecionado.as_deref().map(str::trim_end), Some(PROMPT));
    assert_eq!(painel(&app, 1).selection.row_count(), 1);
    assert_eq!(painel(&app, 0).selection.row_count(), 0);
}

#[test]
fn fechar_painel_devolve_o_espaco() {
    let (app, c) = abrir_com_cmd("um");
    esperar(&c, "o primeiro painel", |_| {
        app.get_panes().row_count() == 1
    });
    let cols = c.app.borrow().panes[&0].cols;
    app.invoke_split_right();

    // Ctrl+Shift+W fecha o painel ativo (o novo).
    assert!(tecla(&app, "W", (true, false, true)));
    assert_eq!(ids(&app), [0]);
    assert_eq!(painel(&app, 0).width, LARGURA);
    assert_eq!(c.app.borrow().panes[&0].cols, cols);

    // O último painel fecha a aba.
    assert!(tecla(&app, "W", (true, false, true)));
    assert!(c.app.borrow().tabs.is_empty());
}

#[test]
fn painel_cujo_processo_termina_some() {
    let (app, c) = abrir_com_cmd("um");
    esperar(&c, "o primeiro painel", |_| {
        app.get_panes().row_count() == 1
    });
    c.app.borrow_mut().shell = Box::new(|| cmd_que_termina("tchau"));
    app.invoke_split_down();
    assert_eq!(ids(&app), [0, 1]);

    esperar(&c, "o painel do processo encerrado sumir", |e| {
        e.panes.len() == 1
    });
    assert_eq!(ids(&app), [0]);
    assert_eq!(painel(&app, 0).height, ALTURA - 32.0);
    assert_eq!(c.app.borrow().tabs.len(), 1, "a aba continua aberta");
}

#[test]
fn nao_divide_painel_pequeno_demais() {
    let (app, c) = abrir_com_cmd("um");
    app.window().set_size(slint::LogicalSize::new(400.0, 300.0));
    esperar(&c, "o terminal encolher", |e| e.panes[&0].cols < 60);

    app.invoke_split_right();
    assert_eq!(ids(&app).len(), 2);
    // Metade de ~200px teria menos de 20 colunas.
    app.invoke_split_right();
    assert_eq!(ids(&app).len(), 2, "não deveria dividir");
}

#[test]
fn nova_aba_tem_um_painel_so() {
    let (app, c) = abrir_com_cmd("um");
    app.invoke_split_right();
    assert_eq!(ids(&app).len(), 2);

    c.app.borrow_mut().new_tab_with(cmd("dois"));
    app.invoke_select_tab(1);
    assert_eq!(ids(&app).len(), 1);
    app.invoke_select_tab(0);
    assert_eq!(ids(&app).len(), 2, "a primeira aba mantém os painéis");
}

// Não verifica nada: gera a captura dos painéis para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let (app, c) = abrir_com_cmd("painel da esquerda");
    esperar_texto(&c, 0, "painel da esquerda");
    app.invoke_split_right();
    esperar_texto(&c, 1, PROMPT);
    digitar(&app, "echo painel de cima\n");
    esperar_texto(&c, 1, "painel de cima");
    app.invoke_split_down();
    esperar_texto(&c, 2, PROMPT);
    digitar(&app, "echo painel de baixo\n");
    esperar_texto(&c, 2, "painel de baixo");
    app.invoke_focus_terminal();
    println!(
        "{}",
        capturar(&app, "AppWindow::term-area", "paineis").display()
    );
    println!(
        "{}",
        capturar(&app, "AppWindow::tab-bar", "abas-paineis").display()
    );
}
