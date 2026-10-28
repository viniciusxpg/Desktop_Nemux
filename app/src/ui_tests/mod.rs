// Testes de interface sem janela: o App roda no backend de testes do Slint,
// com o renderizador por software, e os elementos são achados pelo id
// ("Componente::id", ex.: "VisualPage::gerar"). Um arquivo por página.
//
// As capturas vão para <workspace>/target/ui-capturas/<nome>.png.

mod memoria;
mod paineis;
mod rolagem;
mod selecao;
mod sessoes;
mod terminal;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use i_slint_backend_testing::{ElementHandle, TestingBackend, TestingBackendOptions};
use portable_pty::CommandBuilder;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model};

use crate::AppWindow as App;
use crate::Controllers;
use crate::layout::PaneId;

// tamanho de uma janela típica; o Nemux não tem rolagem de página
pub const LARGURA: f32 = 900.0;
pub const ALTURA: f32 = 500.0;

// Cria a janela no backend de testes. Cada teste roda na sua própria thread,
// com o seu próprio backend.
//
// Sem os controllers: eles abrem shells de verdade e redesenham a partir do
// estado do app. Os testes de visual preenchem as propriedades diretamente;
// os de interação registram só o callback que querem observar.
pub fn abrir() -> App {
    slint::platform::set_platform(Box::new(TestingBackend::new(TestingBackendOptions {
        mock_time: true,
        threading: false,
        renderer_name: Some("software".into()),
    })))
    .expect("backend de testes já iniciado nesta thread");

    let app = App::new().expect("criar o App");
    app.window().set_size(LogicalSize::new(LARGURA, ALTURA));
    app.show().expect("mostrar o App");
    app
}

// Igual a `abrir`, mas com os controllers reais (sem nenhuma aba aberta) e o
// banco de memória em `db`. Nunca use o banco padrão: ele é o do usuário.
pub fn abrir_com_controllers(db: &Path) -> (App, crate::Controllers) {
    let app = abrir();
    let controllers = crate::init_controllers_with_db(&app, db.to_path_buf());
    (app, controllers)
}

pub fn achar(app: &App, id: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_element_id(app, id).next()
}

pub fn elemento(app: &App, id: &str) -> ElementHandle {
    achar(app, id).unwrap_or_else(|| panic!("elemento {id} não encontrado"))
}

// Posição e tamanho em texto, para checar layout sem gastar uma imagem.
pub fn medir(app: &App, id: &str) -> String {
    let e = elemento(app, id);
    let (p, s) = (e.absolute_position(), e.size());
    format!("{id}: x={} y={} {}×{}", p.x, p.y, s.width, s.height)
}

// Renderiza a janela e salva só o retângulo do elemento.
pub fn capturar(app: &App, id: &str, nome: &str) -> PathBuf {
    let tela = app.window().take_snapshot().expect("renderizar a janela");
    let e = elemento(app, id);
    let escala = app.window().scale_factor();
    let (p, s) = (e.absolute_position(), e.size());

    let (largura_tela, altura_tela) = (tela.width() as usize, tela.height() as usize);
    let x0 = ((p.x * escala).max(0.0) as usize).min(largura_tela);
    let y0 = ((p.y * escala).max(0.0) as usize).min(altura_tela);
    let x1 = (((p.x + s.width) * escala).ceil() as usize).min(largura_tela);
    let y1 = (((p.y + s.height) * escala).ceil() as usize).min(altura_tela);
    assert!(x1 > x0 && y1 > y0, "{id} não tem área visível");

    let bytes = tela.as_bytes();
    let mut recorte = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        let linha = (y * largura_tela) * 4;
        recorte.extend_from_slice(&bytes[linha + x0 * 4..linha + x1 * 4]);
    }

    // target/ do workspace (o app fica em app/)
    let pasta = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/ui-capturas");
    fs::create_dir_all(&pasta).expect("criar a pasta das capturas");
    let caminho = pasta.join(format!("{nome}.png"));

    let arquivo = BufWriter::new(File::create(&caminho).expect("criar o PNG"));
    let mut png = png::Encoder::new(arquivo, (x1 - x0) as u32, (y1 - y0) as u32);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .and_then(|mut w| w.write_image_data(&recorte))
        .expect("gravar o PNG");

    caminho
}

// --- Terminais de verdade (cmd.exe no ConPTY) ---

/// Um `cmd.exe` que imprime `texto` (se houver) e continua aberto.
pub fn cmd(texto: &str) -> CommandBuilder {
    let mut cmd = CommandBuilder::new("cmd.exe");
    if texto.is_empty() {
        cmd.arg("/k");
    } else {
        cmd.args(["/k", &format!("echo {texto}")]);
    }
    cmd
}

/// App com controllers e uma aba com `cmd.exe` imprimindo `texto`. Novos painéis
/// e abas também abrem `cmd.exe`. O banco de memória nunca é aberto nestes testes.
pub fn abrir_com_cmd(texto: &str) -> (App, Controllers) {
    let db = std::env::temp_dir().join("nemux-testes-terminal-nao-usado.db");
    let (app, controllers) = abrir_com_controllers(&db);
    {
        let mut estado = controllers.app.borrow_mut();
        estado.shell = Box::new(|| cmd(""));
        estado.new_tab_with(cmd(texto));
    }
    (app, controllers)
}

/// Roda os timers (eventos dos terminais e redesenho) até `pronto` ser verdadeiro.
pub fn esperar(controllers: &Controllers, o_que: &str, pronto: impl Fn(&crate::app::App) -> bool) {
    let limite = Instant::now() + Duration::from_secs(15);
    while Instant::now() < limite {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(20));
        if pronto(&controllers.app.borrow()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("esperei demais por: {o_que}");
}

/// Linha da tela do painel `pane` que começa com `texto`, esperando ela aparecer.
pub fn esperar_texto(controllers: &Controllers, pane: PaneId, texto: &str) -> u16 {
    let linha = |estado: &crate::app::App| {
        estado.panes.get(&pane)?.shown.iter().position(|runs| {
            let linha: String = runs.iter().map(|r| r.text.as_str()).collect();
            linha.starts_with(texto)
        })
    };
    esperar(controllers, &format!("{texto:?} no painel {pane}"), |e| {
        linha(e).is_some()
    });
    linha(&controllers.app.borrow()).unwrap() as u16
}

/// Painel `pane` como a interface o recebeu.
pub fn painel(app: &App, pane: PaneId) -> crate::PaneView {
    app.get_panes()
        .iter()
        .find(|p| p.id as PaneId == pane)
        .unwrap_or_else(|| panic!("painel {pane} não está na tela"))
}

/// Centro de uma célula do painel `pane`, em coordenadas da janela.
pub fn ponto(app: &App, pane: PaneId, linha: u16, coluna: u16) -> LogicalPosition {
    let area = elemento(app, "AppWindow::term-area").absolute_position();
    let p = painel(app, pane);
    let (cw, ch, pad) = (
        app.get_cell_width(),
        app.get_cell_height(),
        app.get_pane_pad(),
    );
    LogicalPosition::new(
        area.x + p.x + pad + (coluna as f32 + 0.5) * cw,
        area.y + p.y + pad + (linha as f32 + 0.5) * ch,
    )
}

pub fn enviar(app: &App, evento: WindowEvent) {
    app.window().dispatch_event(evento);
}

pub fn arrastar(app: &App, de: LogicalPosition, ate: LogicalPosition) {
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

pub fn clicar_em(app: &App, ponto: LogicalPosition) {
    arrastar(app, ponto, ponto);
}
