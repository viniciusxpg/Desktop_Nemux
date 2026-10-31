//! `nemux-mcp statusline`: o comando da barra de status do Claude Code.
//!
//! A cada atualização da tela, o Claude Code roda o comando com um JSON no stdin
//! (sessão, contexto, limites do plano). Gravamos o uso no banco, para o app
//! mostrá-lo, e devolvemos no stdout a linha que o Claude mostra na barra dele.
//! Esse JSON é interno do Claude Code e pode mudar: os campos são todos opcionais.

use std::io::Read;
use std::path::Path;

use nemux_memory::{Limit, Memory, Usage};
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct Status {
    #[serde(default)]
    session_id: String,
    model: Option<Model>,
    context_window: Option<ContextWindow>,
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct Model {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct ContextWindow {
    used_percentage: Option<f64>,
}

#[derive(Deserialize)]
struct RateLimits {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percentage: f64,
    resets_at: i64,
}

impl Window {
    fn limit(&self) -> Limit {
        Limit {
            used_pct: self.used_percentage,
            resets_at: self.resets_at,
        }
    }
}

impl Status {
    fn usage(&self, pane: Option<String>, pid: Option<u32>) -> Usage {
        let limits = self.rate_limits.as_ref();
        Usage {
            session_id: self.session_id.clone(),
            pane,
            pid,
            context_pct: self.context_window.as_ref().and_then(|c| c.used_percentage),
            five_hour: limits.and_then(|l| l.five_hour.as_ref()).map(Window::limit),
            seven_day: limits.and_then(|l| l.seven_day.as_ref()).map(Window::limit),
        }
    }

    /// O que o Claude mostra na barra dele: modelo e contexto (os limites ficam no Nemux).
    fn line(&self) -> String {
        let model = self.model.as_ref().and_then(|m| m.display_name.clone());
        let context = self
            .context_window
            .as_ref()
            .and_then(|c| c.used_percentage)
            .map(|p| format!("contexto {}%", p.round()));
        [model, context]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

pub fn run(db: &Path) -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let status: Status = serde_json::from_str(&input)?;
    if !status.session_id.is_empty() {
        let pane = std::env::var("NEMUX_PANE").ok();
        let usage = status.usage(pane, claude_pid());
        // Um erro no banco não pode apagar a barra do Claude: vai para o stderr.
        if let Err(e) =
            Memory::open(db).and_then(|m| m.save_usage(&usage, nemux_memory::unix_now()))
        {
            eprintln!("nemux-mcp statusline: {e}");
        }
    }
    println!("{}", status.line());
    Ok(())
}

/// O Claude roda o comando por um shell (`claude` → `sh` → este processo): sobe
/// pelos processos pais até achar o Claude.
fn claude_pid() -> Option<u32> {
    let table = process_table();
    let mut pid = std::process::id();
    for _ in 0..6 {
        let parent = table(pid)?.0;
        if is_claude(&table(parent)?.1) {
            return Some(parent);
        }
        pid = parent;
    }
    None
}

fn is_claude(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    name == "claude" || name == "node"
}

/// Pai e nome de um processo, pelo `/proc`.
#[cfg(target_os = "linux")]
fn process_table() -> impl Fn(u32) -> Option<(u32, String)> {
    |pid| {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // "pid (nome) estado ppid ...": o nome pode ter espaços e parênteses.
        let (start, end) = (stat.find('(')?, stat.rfind(')')?);
        let name = stat[start + 1..end].to_string();
        let ppid = stat[end + 1..].split_whitespace().nth(1)?.parse().ok()?;
        Some((ppid, name))
    }
}

/// Pai e nome de cada processo, numa foto tirada agora (Toolhelp).
#[cfg(windows)]
fn process_table() -> impl Fn(u32) -> Option<(u32, String)> {
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    let mut procs = HashMap::new();
    // SAFETY: a foto é conferida antes de usar e fechada no fim; `entry` tem o
    // `dwSize` que a API pede.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap != INVALID_HANDLE_VALUE {
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry) != 0;
            while ok {
                let name = &entry.szExeFile;
                let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                procs.insert(
                    entry.th32ProcessID,
                    (
                        entry.th32ParentProcessID,
                        String::from_utf16_lossy(&name[..len]),
                    ),
                );
                ok = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
    }
    move |pid| procs.get(&pid).cloned()
}

/// Sem um jeito simples de perguntar: o app então não mostra o contexto no painel.
#[cfg(not(any(windows, target_os = "linux")))]
fn process_table() -> impl Fn(u32) -> Option<(u32, String)> {
    |_| None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_status() {
        let json = r#"{
            "session_id": "abc",
            "model": {"id": "claude-opus-5-5", "display_name": "Opus 5.5"},
            "context_window": {"used_percentage": 41.4, "context_window_size": 200000},
            "rate_limits": {
                "five_hour": {"used_percentage": 62, "resets_at": 1790000000},
                "seven_day": {"used_percentage": 18.5, "resets_at": 1790500000}
            },
            "campo_novo": [1, 2]
        }"#;
        let status: Status = serde_json::from_str(json).unwrap();
        let usage = status.usage(Some("p1".into()), Some(7));
        assert_eq!(usage.session_id, "abc");
        assert_eq!(usage.pane.as_deref(), Some("p1"));
        assert_eq!(usage.context_pct, Some(41.4));
        assert_eq!(
            usage.five_hour,
            Some(Limit {
                used_pct: 62.0,
                resets_at: 1_790_000_000
            })
        );
        assert_eq!(usage.seven_day.map(|l| l.used_pct), Some(18.5));
        assert_eq!(status.line(), "Opus 5.5 · contexto 41%");
    }

    #[test]
    fn missing_fields_are_fine() {
        // Antes da primeira resposta não há contexto nem limites.
        let status: Status = serde_json::from_str(
            r#"{"session_id": "abc", "context_window": {"used_percentage": null}}"#,
        )
        .unwrap();
        let usage = status.usage(None, None);
        assert_eq!(usage.context_pct, None);
        assert_eq!(usage.five_hour, None);
        assert_eq!(status.line(), "");
    }

    #[test]
    fn claude_process_names() {
        assert!(is_claude("claude"));
        assert!(is_claude("Claude.EXE"));
        assert!(is_claude("node.exe"));
        assert!(!is_claude("sh"));
        assert!(!is_claude("claude-helper"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_table_reads_parent() {
        let table = process_table();
        let (ppid, name) = table(std::process::id()).unwrap();
        assert!(ppid > 0);
        assert!(!name.is_empty());
    }
}
