// Arquivos soltos na janela: o caminho é colado no painel sob o ponteiro.
//
// O arrastar e soltar de verdade vem do winit, que o backend de testes não tem;
// aqui os testes chamam o que o evento `DroppedFile` chama.

use std::path::PathBuf;

use super::{PROMPT_FIM, abrir_com_cmd, capturar, esperar, esperar_texto, painel};
use crate::Controllers;
use crate::layout::{PaneId, Split};

/// Espera uma linha do painel que contenha `texto`.
fn esperar_linha_com(c: &Controllers, pane: PaneId, texto: &str) {
    esperar(c, &format!("{texto:?} no painel {pane}"), |e| {
        e.panes[&pane].shown.iter().any(|runs| {
            let linha: String = runs.iter().map(|r| r.text.as_str()).collect();
            linha.contains(texto)
        })
    });
}

#[test]
fn sem_ponteiro_vai_para_o_painel_ativo() {
    let (app, c) = abrir_com_cmd("pronto");
    esperar_texto(&c, 0, "pronto");
    let arquivos = [
        PathBuf::from(r"C:\src\main.rs"),
        PathBuf::from(r"C:\Meus Documentos\nota.txt"),
    ];
    c.drop_files(&app, &arquivos, None);
    let colado = r#"C:\src\main.rs "C:\Meus Documentos\nota.txt""#;
    esperar_linha_com(&c, 0, &format!("{PROMPT_FIM}{colado}"));
    assert!(!app.get_file_hover());
}

#[test]
fn vai_para_o_painel_sob_o_ponteiro() {
    let (app, c) = abrir_com_cmd("um");
    esperar_texto(&c, 0, "um");
    assert!(c.app.borrow_mut().split(Split::Right));
    esperar(&c, "o segundo painel", |e| e.panes.len() == 2);
    c.app.borrow_mut().render(&app);
    assert_eq!(c.app.borrow().active_pane_id(), Some(1));

    // O ponteiro sobre o painel da esquerda, que não tem o foco.
    let p = painel(&app, 0);
    let ponto = (
        app.get_area_x() + p.x + p.width / 2.0,
        app.get_area_y() + p.y + p.height / 2.0,
    );
    c.drop_files(&app, &[PathBuf::from(r"C:\tmp\log.txt")], Some(ponto));
    assert_eq!(c.app.borrow().active_pane_id(), Some(0), "ganha o foco");
    esperar_linha_com(&c, 0, &format!(r"{PROMPT_FIM}C:\tmp\log.txt"));

    // Fora da área dos painéis (na barra de abas), vai para o ativo.
    c.drop_files(&app, &[PathBuf::from(r"C:\b.txt")], Some((10.0, 5.0)));
    esperar_linha_com(&c, 0, &format!(r"{PROMPT_FIM}C:\tmp\log.txtC:\b.txt"));
}

#[test]
#[ignore]
fn captura() {
    let (app, c) = abrir_com_cmd("Nemux");
    esperar_texto(&c, 0, "Nemux");
    app.set_file_hover(true);
    println!(
        "{}",
        capturar(&app, "AppWindow::body", "arquivos-soltando").display()
    );
}
