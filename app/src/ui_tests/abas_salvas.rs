// Abas salvas: o que fica gravado ao fechar e o que volta ao abrir de novo.

use std::path::Path;

use nemux_memory::{Memory, PaneSession};
use slint::{ComponentHandle, LogicalSize};

use super::{ALTURA, LARGURA, abrir, cmd, esperar, esperar_texto};
use crate::layout::{Node, Split};
use crate::workspace::{SavedNode, Workspace};
use crate::{AppWindow, Controllers};

/// Controllers gravando as abas em `dir`, com `cmd.exe` no lugar do shell e do Claude.
fn ligar(app: &AppWindow, dir: &Path, config: Option<&Path>) -> Controllers {
    let c = crate::init_controllers_with(
        app,
        dir.join("memoria.db"),
        config,
        Some(&dir.join("session.json")),
    );
    {
        let mut estado = c.app.borrow_mut();
        estado.shell = Box::new(|| cmd(""));
        estado.resume = Box::new(|id, cwd| {
            let mut c = cmd(&format!("retomou {id}"));
            c.cwd(cwd);
            Some(c)
        });
    }
    c
}

/// Outra janela na mesma thread (o backend de testes já foi iniciado por `abrir`).
fn outra_janela() -> AppWindow {
    let app = AppWindow::new().expect("criar o App");
    app.window().set_size(LogicalSize::new(LARGURA, ALTURA));
    app.show().expect("mostrar o App");
    app
}

fn sessao(id: &str, cwd: &Path, pid: u32) -> PaneSession {
    PaneSession {
        session_id: id.into(),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        pid: Some(pid),
    }
}

#[test]
fn grava_e_restaura_abas_paineis_e_claude() {
    let dir = tempfile::tempdir().unwrap();
    let (pasta_a, pasta_b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::create_dir_all(&pasta_a).unwrap();
    std::fs::create_dir_all(&pasta_b).unwrap();

    let app = abrir();
    let c = ligar(&app, dir.path(), None);
    {
        let mut estado = c.app.borrow_mut();
        // Aba 1: um painel numa pasta que o shell informou (no Linux, a pasta é
        // lida do processo: ele já abre nela).
        let mut um = cmd("um");
        um.cwd(&pasta_a);
        estado.new_tab_with(um);
        estado.panes.get_mut(&0).unwrap().cwd = Some(pasta_a.clone());
        estado.rename_tab(0, "API");
        // Aba 2: dois lado a lado, 30% / 70%, com o foco no da esquerda.
        estado.new_tab_with(cmd("dois"));
        assert!(estado.split(Split::Right));
        estado.tabs[1].layout.set_ratio(&[], 0.3);
        estado.focus_pane(1);
    }
    let token = |id| c.app.borrow().panes[&id].token.clone();
    let memoria = Memory::open(dir.path().join("memoria.db")).unwrap();
    let eu = std::process::id();
    // No painel da direita, um Claude, e dentro dele outro (que herdou o painel).
    memoria
        .add_pane_session(&token(2), &sessao("0123abcd-0001", &pasta_b, eu))
        .unwrap();
    memoria
        .add_pane_session(&token(2), &sessao("0123abcd-0002", &pasta_b, eu))
        .unwrap();
    // No da esquerda, um Claude que já fechou sem se desregistrar.
    memoria
        .add_pane_session(&token(1), &sessao("0123abcd-0003", &pasta_b, u32::MAX - 1))
        .unwrap();

    c.save_workspace();
    let salvo = Workspace::load(&dir.path().join("session.json")).expect("session.json");
    assert_eq!(salvo.active_tab, 1);
    assert_eq!(salvo.tabs.len(), 2);
    assert_eq!(
        (salvo.tabs[0].name.as_deref(), salvo.tabs[1].name.as_deref()),
        (Some("API"), None)
    );
    let SavedNode::Pane(um) = &salvo.tabs[0].layout else {
        panic!("a aba 1 tem um painel só");
    };
    assert_eq!(um.cwd.as_deref(), Some(pasta_a.as_path()));
    let SavedNode::Split { down, ratio, a, b } = &salvo.tabs[1].layout else {
        panic!("a aba 2 é dividida");
    };
    assert_eq!((*down, *ratio, salvo.tabs[1].active), (false, 0.3, 0));
    let (SavedNode::Pane(esq), SavedNode::Pane(dir_)) = (&**a, &**b) else {
        panic!("dois painéis");
    };
    assert_eq!(esq.claude_session, None, "o Claude fechado não conta");
    assert_eq!(
        dir_.claude_session.as_deref(),
        Some("0123abcd-0001"),
        "o mais antigo"
    );
    assert_eq!(dir_.cwd.as_deref(), Some(pasta_b.as_path()));
    drop((c, app));

    // Abrir de novo.
    let app = outra_janela();
    let c = ligar(&app, dir.path(), None);
    c.restore_or_open_tab(&app);
    {
        let estado = c.app.borrow();
        assert_eq!((estado.tabs.len(), estado.active), (2, 1));
        assert_eq!(estado.tabs[0].name.as_deref(), Some("API"));
        let Node::Split { split, ratio, a, b } = &estado.tabs[1].layout else {
            panic!("a aba 2 continua dividida");
        };
        assert_eq!((*split, *ratio), (Split::Right, 0.3));
        let (Node::Leaf(esq), Node::Leaf(dir_)) = (&**a, &**b) else {
            panic!("dois painéis");
        };
        assert_eq!(estado.tabs[1].active, *esq);
        assert_eq!(estado.panes[dir_].cwd.as_deref(), Some(pasta_b.as_path()));
        let primeira = estado.tabs[0].layout.leaves()[0];
        assert_eq!(
            estado.panes[&primeira].cwd.as_deref(),
            Some(pasta_a.as_path())
        );
    }
    let direita = c.app.borrow().tabs[1].layout.leaves()[1];
    esperar_texto(&c, direita, "retomou 0123abcd-0001");
}

#[test]
fn sem_abas_salvas_abre_uma_nova() {
    let dir = tempfile::tempdir().unwrap();
    let app = abrir();
    let c = ligar(&app, dir.path(), None);
    c.restore_or_open_tab(&app);
    esperar(&c, "uma aba", |e| e.tabs.len() == 1);

    // Fechar a última aba grava a lista vazia: da próxima vez, uma aba nova.
    c.app.borrow_mut().close_tab(0);
    c.save_workspace();
    let salvo = Workspace::load(&dir.path().join("session.json")).unwrap();
    assert!(salvo.tabs.is_empty());
}

#[test]
fn restore_false_nao_grava_nem_restaura() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "restore = false").unwrap();
    let app = abrir();
    let c = ligar(&app, dir.path(), Some(&config));
    c.app.borrow_mut().new_tab_with(cmd("um"));
    c.save_workspace();
    assert!(!dir.path().join("session.json").exists());
}
