// Avisos: saída nova numa aba escondida e pedidos de atenção (sino) de um painel.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::Model;

use super::{abrir_com_cmd, capturar, esperar, esperar_texto};
use crate::layout::{PaneId, Split};
use crate::{AppWindow, Controllers};

/// Tira o painel do período inicial, em que a saída não conta como novidade.
fn sem_silencio_inicial(c: &Controllers, pane: PaneId) {
    c.app.borrow_mut().panes.get_mut(&pane).unwrap().started =
        Instant::now() - Duration::from_secs(60);
}

fn digitar_no(c: &Controllers, pane: PaneId, texto: &str) {
    c.app.borrow().panes[&pane].term.write(texto.as_bytes());
}

/// Conta as vezes que o app pediu para piscar a barra de tarefas.
fn contar_piscadas(c: &Controllers) -> Rc<Cell<u32>> {
    let n = Rc::new(Cell::new(0));
    c.app.borrow_mut().on_attention = Box::new({
        let n = n.clone();
        move || n.set(n.get() + 1)
    });
    n
}

fn aba(app: &AppWindow, i: usize) -> crate::TabInfo {
    app.get_tabs().row_data(i).expect("aba")
}

#[test]
fn saida_nova_numa_aba_escondida() {
    let (app, c) = abrir_com_cmd("um");
    esperar_texto(&c, 0, "um");
    c.app.borrow_mut().new_tab();
    esperar(&c, "a segunda aba", |e| e.tabs.len() == 2);
    assert!(!aba(&app, 0).activity, "a saída do começo não conta");

    sem_silencio_inicial(&c, 0);
    digitar_no(&c, 0, "echo novidade\r");
    esperar(&c, "a novidade na aba 1", |e| e.panes[&0].activity);
    assert!(aba(&app, 0).activity);
    assert!(!aba(&app, 0).attention);
    assert!(!aba(&app, 1).activity, "a aba à vista não marca");

    let mut estado = c.app.borrow_mut();
    estado.select_tab(0);
    estado.render(&app);
    assert!(!aba(&app, 0).activity, "abrir a aba limpa a marca");
}

#[test]
fn sino_numa_aba_escondida_pede_atencao() {
    let (app, c) = abrir_com_cmd("um");
    esperar_texto(&c, 0, "um");
    c.app.borrow_mut().new_tab();
    esperar(&c, "a segunda aba", |e| e.tabs.len() == 2);
    let piscadas = contar_piscadas(&c);

    digitar_no(&c, 0, "echo \x07\r");
    esperar(&c, "o pedido de atenção", |e| e.panes[&0].attention);
    assert!(aba(&app, 0).attention);
    assert!(piscadas.get() > 0, "pisca a barra de tarefas");

    let mut estado = c.app.borrow_mut();
    estado.select_tab(0);
    estado.render(&app);
    assert!(
        !aba(&app, 0).attention,
        "olhar para o painel limpa o pedido"
    );
}

#[test]
fn sino_no_painel_sem_foco_ou_com_a_janela_atras() {
    let (app, c) = abrir_com_cmd("um");
    esperar_texto(&c, 0, "um");
    assert!(c.app.borrow_mut().split(Split::Right));
    esperar(&c, "o segundo painel", |e| e.panes.len() == 2);

    // O painel da esquerda, sem foco, toca o sino: borda de aviso nele.
    digitar_no(&c, 0, "echo \x07\r");
    esperar(&c, "o pedido no painel sem foco", |e| e.panes[&0].attention);
    let painel = |id: i32| app.get_panes().iter().find(|p| p.id == id).unwrap();
    assert!(painel(0).attention && !painel(1).attention);
    {
        let mut estado = c.app.borrow_mut();
        estado.focus_pane(0);
        estado.render(&app);
    }
    assert!(!painel(0).attention, "dar o foco limpa o pedido");

    // O painel com o foco toca o sino com a janela atrás de outra: marca, até voltar.
    c.app.borrow_mut().foreground = Box::new(|| false);
    digitar_no(&c, 0, "echo \x07\r");
    esperar(&c, "o pedido com a janela atrás", |e| {
        e.panes[&0].attention
    });
    {
        let mut estado = c.app.borrow_mut();
        estado.foreground = Box::new(|| true);
        estado.render(&app);
    }
    assert!(!painel(0).attention);

    // Com a janela na frente e o foco no painel, o sino não marca nem pisca.
    let piscadas = contar_piscadas(&c);
    digitar_no(&c, 0, "echo \x07 visto\r");
    esperar_texto(&c, 0, " visto");
    assert!(!c.app.borrow().panes[&0].attention);
    assert_eq!(piscadas.get(), 0);
}

#[test]
#[ignore]
fn captura() {
    let (app, c) = abrir_com_cmd("um");
    esperar_texto(&c, 0, "um");
    c.app.borrow_mut().new_tab();
    c.app.borrow_mut().new_tab();
    esperar(&c, "três abas", |e| e.tabs.len() == 3);
    assert!(c.app.borrow_mut().split(Split::Right));
    esperar(&c, "o painel dividido", |e| e.panes.len() == 4);
    sem_silencio_inicial(&c, 1);
    digitar_no(&c, 0, "echo \x07\r");
    digitar_no(&c, 1, "echo novidade\r");
    digitar_no(&c, 2, "echo \x07\r");
    esperar(&c, "as marcas", |e| {
        e.panes[&0].attention && e.panes[&1].activity && e.panes[&2].attention
    });
    c.app.borrow_mut().render(&app);
    println!(
        "{}",
        capturar(&app, "AppWindow::tab-bar", "avisos-abas").display()
    );
    println!(
        "{}",
        capturar(&app, "AppWindow::body", "avisos-painel").display()
    );
}
