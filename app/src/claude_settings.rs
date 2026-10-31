//! Liga a barra de status do Claude Code ao Nemux: `statusLine` no `settings.json`
//! do Claude aponta para `nemux-mcp statusline` (ver `usage_view.rs`). O resto do
//! arquivo fica como estava (inclusive a ordem das chaves).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// `settings.json` do usuário: em `CLAUDE_CONFIG_DIR` ou `~/.claude`.
pub fn default_path() -> Option<PathBuf> {
    Some(crate::sessions::claude_dir()?.join("settings.json"))
}

/// A barra de status do Claude já é a do Nemux?
pub fn is_enabled(path: &Path) -> bool {
    read(path).is_ok_and(|settings| settings.get("statusLine").is_some_and(is_ours))
}

/// Aponta a barra de status para `command`. Não troca uma barra que não é do Nemux.
pub fn enable(path: &Path, command: &str) -> Result<(), String> {
    let mut settings = read(path)?;
    if let Some(other) = settings.get("statusLine").filter(|s| !is_ours(s)) {
        let other = other.get("command").and_then(Value::as_str).unwrap_or("?");
        return Err(format!(
            "O Claude Code já tem uma barra de status ({other}) em {}. Tire-a para usar a do Nemux.",
            path.display()
        ));
    }
    settings.insert(
        "statusLine".into(),
        json!({ "type": "command", "command": command, "padding": 0 }),
    );
    write(path, &settings)
}

/// Tira a barra de status, se for a do Nemux.
pub fn disable(path: &Path) -> Result<(), String> {
    let mut settings = read(path)?;
    if settings.get("statusLine").is_some_and(is_ours) {
        settings.remove("statusLine");
        write(path, &settings)?;
    }
    Ok(())
}

/// O comando para o `statusLine`: o `nemux-mcp` junto do executável do Nemux (ou,
/// rodando de `target/debug`, o de `target/release`).
pub fn nemux_command() -> Result<String, String> {
    let name = if cfg!(windows) {
        "nemux-mcp.exe"
    } else {
        "nemux-mcp"
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("executável sem pasta")?;
    let found = [dir.join(name), dir.join("..").join("release").join(name)]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "Não achei o {name} em {}. Compile-o com: cargo build --release -p nemux-mcp",
                dir.display()
            )
        })?;
    Ok(command_for(&found))
}

/// Barras normais também servem no Windows, e não viram escapes no Git Bash
/// (o Claude Code roda o comando por um shell).
fn command_for(exe: &Path) -> String {
    let exe = std::path::absolute(exe).unwrap_or_else(|_| exe.to_path_buf());
    let path = exe.display().to_string().replace('\\', "/");
    format!("\"{path}\" statusline")
}

fn is_ours(status_line: &Value) -> bool {
    status_line
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains("nemux-mcp") && c.ends_with(" statusline"))
}

fn read(path: &Path) -> Result<Map<String, Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("Não foi possível ler {}: {e}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    serde_json::from_str(&text).map_err(|e| format!("{} tem um erro: {e}", path.display()))
}

fn write(path: &Path, settings: &Map<String, Value>) -> Result<(), String> {
    let err = |e: std::io::Error| format!("Não foi possível gravar {}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    // Grava ao lado e troca: o Claude Code relê o arquivo quando ele muda.
    let tmp = path.with_extension("json.nemux-tmp");
    std::fs::write(&tmp, text).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "\"/opt/nemux/nemux-mcp\" statusline";

    #[test]
    fn liga_e_desliga_sem_mexer_no_resto() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            "{\n  \"model\": \"opus\",\n  \"env\": {\"A\": \"1\"}\n}\n",
        )
        .unwrap();
        assert!(!is_enabled(&path));

        enable(&path, CMD).unwrap();
        assert!(is_enabled(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        let keys: Vec<_> = read(&path).unwrap().keys().cloned().collect();
        assert_eq!(keys, ["model", "env", "statusLine"], "{text}");

        disable(&path).unwrap();
        assert!(!is_enabled(&path));
        let keys: Vec<_> = read(&path).unwrap().keys().cloned().collect();
        assert_eq!(keys, ["model", "env"]);
    }

    #[test]
    fn sem_arquivo_cria() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude").join("settings.json");
        enable(&path, CMD).unwrap();
        assert!(is_enabled(&path));
        // Desligar sem a barra do Nemux não faz nada.
        let other = dir.path().join("nada.json");
        disable(&other).unwrap();
        assert!(!other.exists());
    }

    #[test]
    fn nao_troca_barra_de_outro_nem_arquivo_com_erro() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let other = "{\"statusLine\": {\"type\": \"command\", \"command\": \"~/barra.sh\"}}";
        std::fs::write(&path, other).unwrap();
        let err = enable(&path, CMD).unwrap_err();
        assert!(err.contains("~/barra.sh"), "{err}");
        disable(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), other);

        std::fs::write(&path, "{ estragado").unwrap();
        assert!(enable(&path, CMD).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ estragado");
    }

    #[test]
    fn comando_com_aspas_e_barras_normais() {
        let cmd = command_for(Path::new("/opt/nemux/nemux-mcp"));
        assert_eq!(cmd, CMD);
        assert!(is_ours(&json!({ "command": cmd })));
        assert!(!is_ours(&json!({ "command": "nemux-mcp" })));
    }
}
