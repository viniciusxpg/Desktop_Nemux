// Roda do mouse repassada ao programa, com um script Node de verdade (o Claude Code
// também é Node) que liga o mouse ou a tela alternativa e mostra o que recebe.
// Depende do ConPTY novo (vendor/conpty): o do Windows 10 não repassa esses modos.
// Sem `node` no PATH, os testes são pulados.

use portable_pty::CommandBuilder;
use slint::platform::WindowEvent;
use tempfile::TempDir;

use super::{abrir_com_cmd, enviar, esperar, painel, ponto};
use crate::{AppWindow, Controllers};

const SCRIPT: &str = r#"
const modo = process.argv[2];
process.stdin.setRawMode(true);
if (modo === "mouse") process.stdout.write("\x1b[?1000h\x1b[?1006h");
if (modo === "alt") process.stdout.write("\x1b[?1049h");
process.stdout.write("PRONTO\r\n");
process.stdin.on("data", (d) => process.stdout.write("RECEBIDO " + JSON.stringify(d.toString("latin1")) + "\r\n"));
setTimeout(() => process.exit(0), 30000);
"#;

fn tem_node() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Aba com o script no modo pedido; devolve também a pasta do script (precisa viver).
fn abrir_script(modo: &str) -> (AppWindow, Controllers, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("mostra-entrada.js");
    std::fs::write(&script, SCRIPT).unwrap();

    let (app, c) = abrir_com_cmd("");
    let mut node = CommandBuilder::new("node");
    node.args([script.to_str().unwrap(), modo]);
    {
        let mut estado = c.app.borrow_mut();
        estado.close_tab(0);
        estado.new_tab_with(node);
    }
    esperar_conteudo(&c, "PRONTO");
    (app, c, dir)
}

fn texto_do_painel(c: &Controllers) -> String {
    let estado = c.app.borrow();
    let p = &estado.panes[&estado.active_pane_id().unwrap()];
    p.shown
        .iter()
        .map(|runs| runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn esperar_conteudo(c: &Controllers, trecho: &str) {
    esperar(c, &format!("{trecho:?} no painel"), |estado| {
        let Some(id) = estado.active_pane_id() else {
            return false;
        };
        estado.panes[&id].shown.iter().any(|runs| {
            runs.iter()
                .map(|r| r.text.as_str())
                .collect::<String>()
                .contains(trecho)
        })
    });
}

fn rodar_para_cima(app: &AppWindow, c: &Controllers, linha: u16, coluna: u16) {
    let id = c.app.borrow().active_pane_id().unwrap();
    let onde = ponto(app, id, linha, coluna);
    enviar(app, WindowEvent::PointerMoved { position: onde });
    enviar(
        app,
        WindowEvent::PointerScrolled {
            position: onde,
            delta_x: 0.0,
            delta_y: 60.0,
        },
    );
}

#[test]
fn roda_vai_para_o_programa_que_ligou_o_mouse() {
    if !tem_node() {
        eprintln!("node não encontrado: teste pulado");
        return;
    }
    let (app, c, _dir) = abrir_script("mouse");
    rodar_para_cima(&app, &c, 4, 9);

    // SGR: botão 64 (roda para cima), coluna 10, linha 5 (contadas a partir de 1).
    esperar_conteudo(&c, r#"RECEBIDO "\u001b[<64;10;5M""#);
    let id = c.app.borrow().active_pane_id().unwrap();
    assert_eq!(painel(&app, id).offset, 0, "o histórico do Nemux não rola");
}

#[test]
fn tela_alternativa_sem_mouse_recebe_setas() {
    if !tem_node() {
        eprintln!("node não encontrado: teste pulado");
        return;
    }
    let (app, c, _dir) = abrir_script("alt");
    rodar_para_cima(&app, &c, 2, 2);

    esperar_conteudo(&c, r#"\u001b[A"#);
    assert!(
        !texto_do_painel(&c).contains("<64;"),
        "sem mouse ligado, não manda evento de mouse"
    );
}
