fn main() {
    // Os ids dos elementos (usados nos testes de UI) só existem com debug info.
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("falha ao compilar a interface Slint");
}
