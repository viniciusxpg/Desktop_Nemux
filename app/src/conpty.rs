//! ConPTY novo da Microsoft embutido no executável (ver `vendor/conpty/README.md`).
//!
//! O do Windows 10 redesenha a tela em vez de repassar a saída do programa e esconde
//! que o Claude Code usa tela alternativa e mouse. O `portable-pty` carrega um
//! `conpty.dll` pelo nome; se uma DLL com esse nome já estiver carregada, o Windows
//! devolve ela. Por isso os dois arquivos são extraídos para
//! `%LOCALAPPDATA%\nemux\conpty\<versão>\` e o `conpty.dll` de lá é carregado antes
//! do primeiro terminal. Assim o `.exe` funciona sozinho, em qualquer pasta.
//!
//! `NEMUX_SYSTEM_CONPTY=1` desliga isso e usa o ConPTY do Windows (para comparar).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Mesma versão anotada em `vendor/conpty/README.md`; nomeia a pasta da extração.
const VERSION: &str = "1.25.260930003";
const DLL: &[u8] = include_bytes!("../../vendor/conpty/conpty.dll");
const HOST: &[u8] = include_bytes!("../../vendor/conpty/OpenConsole.exe");

/// Pasta de onde o ConPTY novo foi carregado, ou `None` se ficou o do Windows.
pub fn ensure_loaded() -> Option<&'static Path> {
    static LOADED: OnceLock<Option<PathBuf>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            if std::env::var_os("NEMUX_SYSTEM_CONPTY").is_some() {
                return None;
            }
            match load() {
                Ok(dir) => Some(dir),
                Err(e) => {
                    eprintln!("nemux: usando o ConPTY do Windows ({e})");
                    None
                }
            }
        })
        .as_deref()
}

fn load() -> Result<PathBuf, String> {
    let dir = dirs::data_local_dir()
        .ok_or("pasta %LOCALAPPDATA% não encontrada")?
        .join("nemux")
        .join("conpty")
        .join(VERSION);
    std::fs::create_dir_all(&dir).map_err(|e| format!("criar {}: {e}", dir.display()))?;
    extract(&dir.join("conpty.dll"), DLL)?;
    extract(&dir.join("OpenConsole.exe"), HOST)?;

    let dll = dir.join("conpty.dll");
    // SAFETY: é a DLL oficial da Microsoft embutida no executável; ela só exporta as
    // funções de pseudoconsole e não roda nada ao ser carregada além do próprio DllMain.
    let lib = unsafe { libloading::Library::new(&dll) }
        .map_err(|e| format!("carregar {}: {e}", dll.display()))?;
    // Fica carregada até o fim do processo, para o `portable-pty` a encontrar pelo nome.
    std::mem::forget(lib);
    Ok(dir)
}

/// Grava `bytes` em `path`, a menos que já esteja lá. Outro Nemux aberto pode estar
/// usando o arquivo: grava num temporário e renomeia, e um arquivo igual já basta.
fn extract(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let same = |p: &Path| std::fs::read(p).is_ok_and(|current| current == bytes);
    if same(path) {
        return Ok(());
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let result = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    let _ = std::fs::remove_file(&tmp);
    match result {
        Ok(()) => Ok(()),
        Err(_) if same(path) => Ok(()),
        Err(e) => Err(format!("gravar {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_writes_once_and_keeps_equal_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.bin");
        extract(&path, b"abc").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");

        // Conteúdo diferente (versão antiga) é trocado.
        extract(&path, b"novo").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"novo");
        // Nenhum temporário fica para trás.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn loads_the_embedded_conpty() {
        if std::env::var_os("NEMUX_SYSTEM_CONPTY").is_some() {
            return;
        }
        let dir = ensure_loaded().expect("o ConPTY embutido deveria carregar");
        assert_eq!(std::fs::read(dir.join("conpty.dll")).unwrap(), DLL);
        assert_eq!(std::fs::read(dir.join("OpenConsole.exe")).unwrap(), HOST);
    }
}
