use std::fs::File;
use std::path::{Path, PathBuf};

fn main() {
    // Os ids dos elementos (usados nos testes de UI) só existem com debug info.
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("falha ao compilar a interface Slint");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon();
    }
}

/// Gera o `.ico` a partir dos SVGs em `ui/icons` e o embute no executável
/// (ícone no Explorer e na barra de tarefas).
fn windows_icon() {
    println!("cargo:rerun-if-changed=ui/icons");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico_path = out.join("nemux.ico");

    let big = load_svg(Path::new("ui/icons/nemux.svg"));
    // Em 16 e 24 px o desenho normal fica fino demais.
    let small = load_svg(Path::new("ui/icons/nemux-small.svg"));

    let mut icon = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 20, 24, 32, 40, 48, 64, 128, 256] {
        let tree = if size <= 24 { &small } else { &big };
        let image = ico::IconImage::from_rgba_data(size, size, render(tree, size));
        icon.add_entry(ico::IconDirEntry::encode_as_png(&image).expect("codificar o ícone"));
    }
    icon.write(File::create(&ico_path).expect("criar o .ico"))
        .expect("gravar o .ico");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path.to_str().unwrap());
    // Sem o compilador de recursos (rc.exe do Windows SDK) o app ainda funciona,
    // só fica sem ícone no Explorer.
    if let Err(e) = res.compile() {
        println!("cargo:warning=ícone do executável não embutido: {e}");
    }
}

fn load_svg(path: &Path) -> resvg::usvg::Tree {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("ler {}: {e}", path.display()));
    resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .unwrap_or_else(|e| panic!("SVG inválido em {}: {e}", path.display()))
}

/// Rasteriza o SVG num quadrado de `size` px e devolve RGBA sem pré-multiplicação.
fn render(tree: &resvg::usvg::Tree, size: u32) -> Vec<u8> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
    let scale = size as f32 / tree.size().width();
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}
