// Testes de interface sem janela: o App roda no backend de testes do Slint,
// com o renderizador por software, e os elementos são achados pelo id
// ("Componente::id", ex.: "VisualPage::gerar"). Um arquivo por página.
//
// As capturas vão para <workspace>/target/ui-capturas/<nome>.png.

mod memoria;
mod selecao;
mod terminal;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use i_slint_backend_testing::{ElementHandle, TestingBackend, TestingBackendOptions};
use slint::{ComponentHandle, LogicalSize};

use crate::AppWindow as App;

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
