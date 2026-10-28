// Painel de memória sobre um banco temporário (nunca o nemux.db do usuário).

use std::path::PathBuf;
use std::time::Duration;

use i_slint_backend_testing::ElementHandle;
use nemux_memory::Memory;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model};
use tempfile::TempDir;

use super::{abrir_com_controllers, capturar, medir};
use crate::{AppWindow, Controllers};

struct Cenario {
    app: AppWindow,
    _controllers: Controllers,
    db: PathBuf,
    _dir: TempDir,
}

/// Banco com três memórias em dois projetos; o painel começa fechado.
fn cenario() -> Cenario {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("nemux.db");
    let mem = Memory::open(&db).unwrap();
    mem.save("nemux", "A interface usa Slint 1.18").unwrap();
    mem.save("nemux", "O usuário prefere respostas em português")
        .unwrap();
    mem.save("outro", "Deploy roda com cargo dist").unwrap();
    drop(mem);

    let (app, controllers) = abrir_com_controllers(&db);
    Cenario {
        app,
        _controllers: controllers,
        db,
        _dir: dir,
    }
}

fn botao(app: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .unwrap_or_else(|| panic!("botão {label:?} não encontrado"))
}

fn abrir_painel(app: &AppWindow) {
    botao(app, "Memória").invoke_accessible_default_action();
    assert!(app.get_memory_open());
}

fn conteudos(app: &AppWindow) -> Vec<String> {
    app.get_memory_entries()
        .iter()
        .map(|e| e.content.to_string())
        .collect()
}

fn buscar(app: &AppWindow, texto: &str) {
    app.set_memory_query(texto.into());
    app.invoke_memory_refresh();
}

#[test]
fn abre_e_lista_as_mais_recentes() {
    let c = cenario();
    assert!(super::achar(&c.app, "AppWindow::memory-panel").is_none());
    abrir_painel(&c.app);

    assert_eq!(
        conteudos(&c.app),
        [
            "Deploy roda com cargo dist",
            "O usuário prefere respostas em português",
            "A interface usa Slint 1.18"
        ]
    );
    let projetos: Vec<_> = c
        .app
        .get_memory_projects()
        .iter()
        .map(|p| p.to_string())
        .collect();
    assert_eq!(projetos, ["Todos os projetos", "outro (1)", "nemux (2)"]);
    assert!(c.app.get_memory_status().starts_with("3 memórias · "));
}

#[test]
fn busca_e_filtro_por_projeto() {
    let c = cenario();
    abrir_painel(&c.app);

    buscar(&c.app, "portugues");
    assert_eq!(
        conteudos(&c.app),
        ["O usuário prefere respostas em português"]
    );
    assert!(c.app.get_memory_status().starts_with("1 encontradas de 3"));

    buscar(&c.app, "");
    c.app.set_memory_project_index(2); // nemux
    c.app.invoke_memory_refresh();
    assert_eq!(c.app.get_memory_entries().row_count(), 2);
    assert!(c.app.get_memory_status().starts_with("2 memórias"));

    // O filtro continua no mesmo projeto mesmo que a ordem da lista mude.
    Memory::open(&c.db)
        .unwrap()
        .save("terceiro", "novo projeto")
        .unwrap();
    c.app.invoke_memory_refresh();
    let projetos: Vec<_> = c
        .app
        .get_memory_projects()
        .iter()
        .map(|p| p.to_string())
        .collect();
    assert_eq!(
        projetos[c.app.get_memory_project_index() as usize],
        "nemux (2)"
    );
}

#[test]
fn apagar_pede_confirmacao() {
    let c = cenario();
    abrir_painel(&c.app);
    let id = c.app.get_memory_entries().row_data(0).unwrap().id;

    botao(&c.app, &format!("Apagar memória {id}")).invoke_accessible_default_action();
    assert_eq!(
        c.app.get_memory_entries().row_count(),
        3,
        "o primeiro clique só pede confirmação"
    );

    botao(&c.app, &format!("Confirmar apagar memória {id}")).invoke_accessible_default_action();
    assert_eq!(c.app.get_memory_entries().row_count(), 2);
    assert_eq!(
        Memory::open(&c.db).unwrap().recent(None, 10).unwrap().len(),
        2
    );
}

#[test]
fn mostra_o_que_outra_sessao_salvar() {
    let c = cenario();
    abrir_painel(&c.app);

    // Outra conexão, como o servidor MCP de uma sessão do Claude.
    Memory::open(&c.db)
        .unwrap()
        .save("nemux", "salvo pelo Claude agora")
        .unwrap();
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1100));
    assert_eq!(conteudos(&c.app)[0], "salvo pelo Claude agora");
}

#[test]
fn atalho_e_esc_abrem_e_fecham() {
    let c = cenario();
    c.app.invoke_focus_terminal();
    let tecla = |e: WindowEvent| c.app.window().dispatch_event(e);
    let (ctrl, shift) = (Key::Control.into(), Key::Shift.into());

    tecla(WindowEvent::KeyPressed { text: ctrl });
    tecla(WindowEvent::KeyPressed { text: shift });
    tecla(WindowEvent::KeyPressed { text: "M".into() });
    tecla(WindowEvent::KeyReleased { text: "M".into() });
    tecla(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    tecla(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    assert!(c.app.get_memory_open());
    assert_eq!(c.app.get_memory_entries().row_count(), 3);

    // O campo de busca recebe o foco; o Esc sobe até o painel e o fecha.
    tecla(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    tecla(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!c.app.get_memory_open());
}

#[test]
fn erro_ao_abrir_o_banco_aparece_no_painel() {
    let dir = tempfile::tempdir().unwrap();
    // Um arquivo no lugar da pasta: o banco não pode ser criado.
    let bloqueio = dir.path().join("arquivo");
    std::fs::write(&bloqueio, "").unwrap();
    let (app, _controllers) = abrir_com_controllers(&bloqueio.join("nemux.db"));

    abrir_painel(&app);
    assert!(
        app.get_memory_status()
            .starts_with("Não foi possível abrir")
    );
    assert_eq!(app.get_memory_entries().row_count(), 0);
}

// Não verifica nada: gera as capturas e as medidas para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let c = cenario();
    abrir_painel(&c.app);
    let id = c.app.get_memory_entries().row_data(1).unwrap().id;
    botao(&c.app, &format!("Apagar memória {id}")).invoke_accessible_default_action();

    println!("{}", medir(&c.app, "AppWindow::memory-panel"));
    println!("{}", medir(&c.app, "AppWindow::term-area"));
    println!("grade {}×{}", c.app.get_grid_cols(), c.app.get_grid_rows());
    println!(
        "{}",
        capturar(&c.app, "AppWindow::memory-panel", "memoria").display()
    );
    println!(
        "{}",
        capturar(&c.app, "AppWindow::tab-bar", "abas-memoria").display()
    );
}
