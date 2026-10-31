// Uso do Claude Code (usage_view.rs): limites do plano na barra de abas, contexto
// no painel onde o Claude roda e a opção "Uso do Claude na barra" da engrenagem.

use i_slint_backend_testing::ElementHandle;
use nemux_memory::{Limit, Memory, Usage, unix_now};
use slint::Model;

use super::{abrir_com_controllers, achar, capturar, cmd, esperar};
use crate::AppWindow;

fn botao(app: &AppWindow, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label).next()
}

const OPCAO: &str = "Uso do Claude na barra";

#[test]
fn limites_na_barra_e_contexto_no_painel() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memoria.db");
    let memoria = Memory::open(&db).unwrap();
    let agora = unix_now();
    let limite = |used_pct, resets_at| {
        Some(Limit {
            used_pct,
            resets_at,
        })
    };
    let conta = Usage {
        session_id: "s1".into(),
        // Folga de 30 s: o texto continua "1 h" enquanto o teste roda.
        five_hour: limite(62.0, agora + 3600 + 30),
        seven_day: limite(84.0, agora + 3 * 86_400 + 30),
        ..Usage::default()
    };
    memoria.save_usage(&conta, agora).unwrap();

    let (app, c) = abrir_com_controllers(&db);
    let medidores: Vec<_> = app
        .get_usage_meters()
        .iter()
        .map(|m| (m.label.to_string(), m.percent))
        .collect();
    assert_eq!(
        medidores,
        [("5h".to_string(), 62.0), ("7d".to_string(), 84.0)]
    );
    assert_eq!(
        app.get_usage_detail(),
        "Limite de 5 horas: 62% usado, zera em 1 h\nLimite semanal: 84% usado, zera em 3 dias"
    );
    // O detalhe só aparece com o mouse sobre os limites.
    assert!(achar(&app, "AppWindow::usage-tip").is_none());

    // Um Claude (este processo, que está vivo) gravando o uso pelo painel 0.
    c.app.borrow_mut().new_tab_with(cmd("um"));
    let token = c.app.borrow().panes[&0].token.clone();
    let sessao = Usage {
        session_id: "s2".into(),
        pane: Some(token),
        pid: Some(std::process::id()),
        context_pct: Some(41.4),
        ..Usage::default()
    };
    memoria.save_usage(&sessao, agora).unwrap();
    esperar(&c, "o contexto aparecer no painel", |_| {
        app.get_panes().iter().any(|p| p.id == 0 && p.context == 41)
    });
    assert!(achar(&app, "TerminalPane::context-badge").is_some());
}

#[test]
fn opcao_do_menu_liga_e_desliga_a_barra_do_claude() {
    let dir = tempfile::tempdir().unwrap();
    let (app, c) = abrir_com_controllers(&dir.path().join("memoria.db"));
    botao(&app, "Configurações")
        .unwrap()
        .invoke_accessible_default_action();
    // Sem o settings.json do Claude (os testes não mexem no do usuário), não aparece.
    assert!(botao(&app, OPCAO).is_none());

    let settings = dir.path().join("settings.json");
    let nosso = r#"{"model": "opus", "statusLine": {"type": "command", "command": "\"/x/nemux-mcp\" statusline"}}"#;
    std::fs::write(&settings, nosso).unwrap();
    c.use_claude_settings(&app, settings.clone());
    assert!(app.get_usage_bar(), "a barra do Claude já é a do Nemux");

    // Desmarcar tira a barra do Nemux e deixa o resto.
    botao(&app, OPCAO)
        .unwrap()
        .invoke_accessible_default_action();
    assert!(!app.get_usage_bar());
    let texto = std::fs::read_to_string(&settings).unwrap();
    let json: serde_json::Value = serde_json::from_str(&texto).unwrap();
    assert_eq!(json, serde_json::json!({ "model": "opus" }));

    // Marcar sem um nemux-mcp junto do executável (o dos testes fica em
    // target/debug/deps) avisa e não grava nada.
    botao(&app, OPCAO)
        .unwrap()
        .invoke_accessible_default_action();
    assert!(!app.get_usage_bar());
    assert!(app.get_config_error().contains("nemux-mcp"));
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), texto);
}

// Não verifica nada: gera a captura da barra de uso para conferência visual.
// Fica fora do cargo test normal; rode com `cargo capturas`.
#[test]
#[ignore = "só gera capturas; rode com cargo capturas"]
fn capturas() {
    let app = super::abrir();
    let medidor = |label: &str, percent| crate::UsageMeter {
        label: label.into(),
        percent,
    };
    let medidores = vec![medidor("5h", 62.0), medidor("7d", 84.0)];
    app.set_usage_meters(slint::ModelRc::new(slint::VecModel::from(medidores)));
    println!(
        "{}",
        capturar(&app, "AppWindow::tab-bar", "uso-barra").display()
    );
}
