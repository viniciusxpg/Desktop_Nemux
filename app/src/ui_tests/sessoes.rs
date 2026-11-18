// "Retomar sessão do Claude" sobre sessões falsas numa pasta temporária. Retomar
// abre um cmd.exe que imprime o id, em vez de rodar o `claude` de verdade.

use std::path::Path;

use i_slint_backend_testing::ElementHandle;
use slint::Model;
use slint::platform::{Key, WindowEvent};
use tempfile::TempDir;

use super::{abrir_com_cmd, capturar, cmd, enviar, esperar, esperar_texto};
use crate::{AppWindow, Controllers};

const ID_NOVA: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
const ID_SEM_PASTA: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
const ID_SEM_PASTA_2: &str = "33333333-aaaa-4bbb-8ccc-000000000003";
const ID_EM_USO_SEM_PASTA: &str = "44444444-aaaa-4bbb-8ccc-000000000004";

struct Cenario {
    app: AppWindow,
    c: Controllers,
    _dir: TempDir,
}

fn sessao(dir: &Path, id: &str, cwd: &Path, titulo: &str, minutos_atras: u64) {
    let pasta = dir.join("projeto");
    std::fs::create_dir_all(&pasta).unwrap();
    let arquivo = pasta.join(format!("{id}.jsonl"));
    let linhas = [
        serde_json::json!({"type": "user", "cwd": cwd, "message": {"content": "primeira mensagem"}}),
        serde_json::json!({"type": "ai-title", "aiTitle": titulo}),
    ];
    let texto: String = linhas.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&arquivo, texto).unwrap();
    let quando = std::time::SystemTime::now() - std::time::Duration::from_secs(minutos_atras * 60);
    std::fs::File::options()
        .write(true)
        .open(&arquivo)
        .unwrap()
        .set_modified(quando)
        .unwrap();
}

/// Uma aba aberta e duas sessões: uma retomável (gravada agora, "em uso") e uma
/// cuja pasta não existe mais.
fn cenario() -> Cenario {
    let dir = tempfile::tempdir().unwrap();
    let projeto = dir.path().join("meu-projeto");
    std::fs::create_dir_all(&projeto).unwrap();
    sessao(dir.path(), ID_NOVA, &projeto, "Corrigir o build", 0);
    sessao(
        dir.path(),
        ID_SEM_PASTA,
        &dir.path().join("apagada"),
        "Projeto antigo",
        90,
    );

    let (app, c) = abrir_com_cmd("base");
    {
        let mut sessoes = c.sessions.borrow_mut();
        sessoes.dir = Some(dir.path().to_path_buf());
        sessoes.command_for =
            Box::new(|s| s.is_available().then(|| cmd(&format!("retomada {}", s.id))));
    }
    esperar(&c, "a primeira aba na tela", |_| {
        app.get_tabs().row_count() == 1
    });
    Cenario { app, c, _dir: dir }
}

fn botao(app: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .unwrap_or_else(|| panic!("botão {label:?} não encontrado"))
}

fn titulos(app: &AppWindow) -> Vec<String> {
    app.get_sessions()
        .iter()
        .map(|s| s.title.to_string())
        .collect()
}

#[test]
fn lista_as_sessoes_mais_recentes_primeiro() {
    let s = cenario();
    botao(&s.app, "Retomar sessão do Claude (Ctrl+Shift+R)").invoke_accessible_default_action();
    assert!(s.app.get_sessions_open());

    assert_eq!(titulos(&s.app), ["Corrigir o build", "Projeto antigo"]);
    let (nova, antiga) = (
        s.app.get_sessions().row_data(0).unwrap(),
        s.app.get_sessions().row_data(1).unwrap(),
    );
    assert!(nova.available && nova.active, "gravada agora: em uso");
    assert_eq!(nova.when, "agora");
    assert!(nova.folder.ends_with("meu-projeto"));
    assert!(!antiga.available && !antiga.active);
    assert_eq!(antiga.when, "há 1 h");
    assert!(s.app.get_sessions_status().starts_with("2 sessões."));
}

#[test]
fn filtro_por_titulo_ou_pasta() {
    let s = cenario();
    s.app.invoke_toggle_sessions();

    s.app.set_sessions_query("BUILD".into());
    s.app.invoke_sessions_filter();
    assert_eq!(titulos(&s.app), ["Corrigir o build"]);

    s.app.set_sessions_query("apagada".into());
    s.app.invoke_sessions_filter();
    assert_eq!(titulos(&s.app), ["Projeto antigo"]);

    s.app.set_sessions_query("nada disso".into());
    s.app.invoke_sessions_filter();
    assert!(titulos(&s.app).is_empty());
}

#[test]
fn retomar_abre_uma_aba_nova_na_sessao() {
    let s = cenario();
    s.app.invoke_toggle_sessions();
    botao(&s.app, "Retomar Corrigir o build").invoke_accessible_default_action();

    assert!(!s.app.get_sessions_open(), "a lista fecha");
    let estado = s.c.app.borrow();
    assert_eq!(
        (estado.tabs.len(), estado.active),
        (2, 1),
        "uma aba nova, ativa"
    );
    let painel = estado.tabs[1].active;
    drop(estado);
    esperar_texto(&s.c, painel, &format!("retomada {ID_NOVA}"));
}

#[test]
fn sessao_sem_pasta_nao_abre() {
    let s = cenario();
    s.app.invoke_toggle_sessions();
    s.app.invoke_session_open(1);

    assert!(s.app.get_sessions_open(), "a lista continua aberta");
    assert!(
        s.app
            .get_sessions_status()
            .contains("pasta dela não existe")
    );
    assert_eq!(s.c.app.borrow().tabs.len(), 1);
}

#[test]
fn atalho_abre_e_esc_fecha() {
    let s = cenario();
    s.app.invoke_focus_terminal();
    let tecla = |texto: slint::SharedString| {
        enviar(
            &s.app,
            WindowEvent::KeyPressed {
                text: texto.clone(),
            },
        );
        enviar(&s.app, WindowEvent::KeyReleased { text: texto });
    };
    enviar(
        &s.app,
        WindowEvent::KeyPressed {
            text: Key::Control.into(),
        },
    );
    enviar(
        &s.app,
        WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        },
    );
    tecla("R".into());
    enviar(
        &s.app,
        WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        },
    );
    enviar(
        &s.app,
        WindowEvent::KeyReleased {
            text: Key::Control.into(),
        },
    );
    assert!(s.app.get_sessions_open());
    assert_eq!(titulos(&s.app).len(), 2);

    // O campo de filtro tem o foco; o Esc sobe até a lista e a fecha.
    tecla(Key::Escape.into());
    assert!(!s.app.get_sessions_open());
}

#[test]
fn sem_pasta_de_sessoes_avisa() {
    let s = cenario();
    s.c.sessions.borrow_mut().dir = None;
    s.app.invoke_toggle_sessions();
    assert!(titulos(&s.app).is_empty());
    assert_eq!(
        s.app.get_sessions_status(),
        "Pasta de sessões do Claude Code não encontrada."
    );
}

#[test]
fn excluir_pede_confirmacao_e_apaga_do_disco() {
    let s = cenario();
    let arquivo = s
        ._dir
        .path()
        .join("projeto")
        .join(format!("{ID_SEM_PASTA}.jsonl"));
    s.app.invoke_toggle_sessions();

    botao(&s.app, "Excluir Projeto antigo").invoke_accessible_default_action();
    assert!(arquivo.exists(), "a lixeira só pede confirmação");
    botao(&s.app, "Cancelar exclusão de Projeto antigo").invoke_accessible_default_action();
    assert!(arquivo.exists());
    assert_eq!(titulos(&s.app).len(), 2);

    botao(&s.app, "Excluir Projeto antigo").invoke_accessible_default_action();
    botao(&s.app, "Confirmar exclusão de Projeto antigo").invoke_accessible_default_action();
    assert!(!arquivo.exists());
    assert_eq!(titulos(&s.app), ["Corrigir o build"]);
    assert_eq!(
        s.app.get_sessions_status(),
        "Sessão \"Projeto antigo\" excluída."
    );
    assert!(s.app.get_sessions_open(), "a lista continua aberta");
}

#[test]
fn sessao_em_uso_nao_se_exclui() {
    let s = cenario();
    let arquivo = s
        ._dir
        .path()
        .join("projeto")
        .join(format!("{ID_NOVA}.jsonl"));
    s.app.invoke_toggle_sessions();

    assert!(
        ElementHandle::find_by_accessible_label(&s.app, "Excluir Corrigir o build")
            .next()
            .is_none(),
        "sem lixeira na sessão em uso"
    );
    // Nem pelo callback.
    s.app.invoke_session_delete(0);
    assert!(arquivo.exists());
    assert!(s.app.get_sessions_status().contains("está em uso"));
}

#[test]
fn excluir_as_sem_pasta_de_uma_vez() {
    let s = cenario();
    let projeto = s._dir.path().join("projeto");
    // Mais uma sem pasta, e uma em uso sem pasta, que fica.
    let apagada = s._dir.path().join("apagada");
    sessao(s._dir.path(), ID_SEM_PASTA_2, &apagada, "Outra antiga", 300);
    sessao(
        s._dir.path(),
        ID_EM_USO_SEM_PASTA,
        &apagada,
        "Rodando ainda",
        0,
    );
    s.app.invoke_toggle_sessions();
    assert_eq!(s.app.get_sessions_orphans(), 2, "a em uso não conta");

    botao(&s.app, "Excluir as sessões sem pasta").invoke_accessible_default_action();
    assert_eq!(titulos(&s.app).len(), 4, "só pede confirmação");
    botao(&s.app, "Cancelar exclusão das sessões sem pasta").invoke_accessible_default_action();
    assert_eq!(titulos(&s.app).len(), 4);

    botao(&s.app, "Excluir as sessões sem pasta").invoke_accessible_default_action();
    botao(&s.app, "Confirmar exclusão das sessões sem pasta").invoke_accessible_default_action();
    assert_eq!(titulos(&s.app), ["Rodando ainda", "Corrigir o build"]);
    assert!(!projeto.join(format!("{ID_SEM_PASTA}.jsonl")).exists());
    assert!(!projeto.join(format!("{ID_SEM_PASTA_2}.jsonl")).exists());
    assert_eq!(s.app.get_sessions_orphans(), 0);
    assert_eq!(
        s.app.get_sessions_status(),
        "2 sessões sem pasta excluídas."
    );
    assert!(
        ElementHandle::find_by_accessible_label(&s.app, "Excluir as sessões sem pasta")
            .next()
            .is_none(),
        "sem sessões sem pasta, o botão some"
    );
}

// Não verifica nada: gera a captura da lista para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let s = cenario();
    // A barra antes de abrir a lista, que escurece o resto da janela.
    println!(
        "{}",
        capturar(&s.app, "AppWindow::top-bar", "barra-sessoes").display()
    );
    s.app.invoke_toggle_sessions();
    println!(
        "{}",
        capturar(&s.app, "AppWindow::session-picker", "sessoes").display()
    );
    botao(&s.app, "Excluir Projeto antigo").invoke_accessible_default_action();
    println!(
        "{}",
        capturar(&s.app, "AppWindow::session-picker", "sessoes-excluir").display()
    );
    botao(&s.app, "Excluir as sessões sem pasta").invoke_accessible_default_action();
    println!(
        "{}",
        capturar(
            &s.app,
            "AppWindow::session-picker",
            "sessoes-excluir-sem-pasta"
        )
        .display()
    );
}
