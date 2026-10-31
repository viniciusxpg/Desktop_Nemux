//! Abas e painéis entre uma execução e outra: `%APPDATA%\nemux\session.json`.
//!
//! Guarda a árvore de cada aba, a pasta de cada painel e, se um Claude roda nele,
//! a sessão (que o `nemux-mcp` registra no banco), para retomá-la com `--resume`.

use std::path::{Path, PathBuf};

use nemux_memory::{Memory, PaneSession};
use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};

use crate::app::{App, Pane};
use crate::layout::{Node, Split};
#[cfg(test)]
use crate::sessions;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub active_tab: usize,
    pub tabs: Vec<SavedTab>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    /// Nome dado pelo usuário; sem ele, a aba mostra o título do programa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Painel com o foco, na ordem de leitura dos painéis da aba.
    pub active: usize,
    pub layout: SavedNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedNode {
    Pane(SavedPane),
    Split {
        /// `true` = um abaixo do outro; `false` = lado a lado.
        down: bool,
        ratio: f32,
        a: Box<SavedNode>,
        b: Box<SavedNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SavedPane {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Sessão do Claude Code aberta no painel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session: Option<String>,
}

/// Onde o app guarda as abas.
pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nemux")
        .join("session.json")
}

impl Workspace {
    /// O estado de `app`; `claude_of` diz qual sessão do Claude roda em cada painel.
    pub fn capture(app: &App, claude_of: impl Fn(&Pane) -> Option<PaneSession>) -> Self {
        let pane = |id| {
            let Some(pane) = app.panes.get(&id) else {
                return SavedPane::default();
            };
            // A do sistema (Linux) é a mais atual; senão, a que o shell informou.
            let cwd = || pane.term.cwd().or_else(|| pane.cwd.clone());
            match claude_of(pane) {
                Some(s) => SavedPane {
                    cwd: s.cwd.map(PathBuf::from).or_else(cwd),
                    claude_session: Some(s.session_id),
                },
                None => SavedPane {
                    cwd: cwd(),
                    claude_session: None,
                },
            }
        };
        let tabs = app
            .tabs
            .iter()
            .map(|tab| SavedTab {
                name: tab.name.clone(),
                active: tab
                    .layout
                    .leaves()
                    .iter()
                    .position(|&id| id == tab.active)
                    .unwrap_or(0),
                layout: save_node(&tab.layout, &pane),
            })
            .collect();
        Self {
            active_tab: app.active,
            tabs,
        }
    }

    /// Lê `path`; sem arquivo (ou com um arquivo estragado), `None`.
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text)
            .inspect_err(|e| eprintln!("nemux: {} ignorado: {e}", path.display()))
            .ok()
    }

    /// Grava num arquivo temporário e troca, para não deixar um arquivo pela metade.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)
    }
}

fn save_node(node: &Node, pane: &impl Fn(u64) -> SavedPane) -> SavedNode {
    match node {
        Node::Leaf(id) => SavedNode::Pane(pane(*id)),
        Node::Split { split, ratio, a, b } => SavedNode::Split {
            down: *split == Split::Down,
            ratio: *ratio,
            a: Box::new(save_node(a, pane)),
            b: Box::new(save_node(b, pane)),
        },
    }
}

/// A sessão do Claude que roda no painel: a mais antiga das que ainda estão abertas
/// (um Claude aberto por outro, como um `claude -p` da ferramenta Bash, herda o painel).
pub fn claude_session(memory: &Memory, pane: &Pane) -> Option<PaneSession> {
    let sessions = memory
        .pane_sessions(&pane.token)
        .inspect_err(|e| eprintln!("nemux: não foi possível ler as sessões do painel: {e}"))
        .ok()?;
    sessions
        .into_iter()
        .find(|s| s.pid.is_none_or(process_alive))
}

/// Retoma a sessão `id` do Claude na pasta: `sessions::resume_in`, ou outro nos testes.
pub type Resume = dyn Fn(&str, &Path) -> Option<CommandBuilder>;

/// Comando que reabre um painel salvo: o Claude retomado na pasta dele, ou o shell.
pub fn command_for(
    saved: &SavedPane,
    shell: &dyn Fn() -> CommandBuilder,
    resume: &Resume,
) -> CommandBuilder {
    let cwd = saved.cwd.as_deref().filter(|d| d.is_dir());
    if let (Some(id), Some(cwd)) = (&saved.claude_session, cwd)
        && let Some(cmd) = resume(id, cwd)
    {
        return cmd;
    }
    let mut cmd = shell();
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }
    cmd
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: o handle é conferido antes de usar e fechado logo depois.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE as u32
    }
}

#[cfg(target_os = "linux")]
fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Sem um jeito simples de perguntar, conta como vivo (no pior caso, retoma uma
/// sessão que já fechou).
#[cfg(not(any(windows, target_os = "linux")))]
fn process_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(cwd: &str) -> SavedNode {
        SavedNode::Pane(SavedPane {
            cwd: Some(cwd.into()),
            claude_session: None,
        })
    }

    #[test]
    fn grava_e_le() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nemux").join("session.json");
        assert_eq!(Workspace::load(&path), None);

        let ws = Workspace {
            active_tab: 1,
            tabs: vec![
                SavedTab {
                    name: Some("API".into()),
                    active: 0,
                    layout: pane("C:/a"),
                },
                SavedTab {
                    name: None,
                    active: 1,
                    layout: SavedNode::Split {
                        down: false,
                        ratio: 0.3,
                        a: Box::new(pane("C:/b")),
                        b: Box::new(SavedNode::Pane(SavedPane {
                            cwd: Some("C:/c".into()),
                            claude_session: Some("abc-123".into()),
                        })),
                    },
                },
            ],
        };
        ws.save(&path).unwrap();
        assert_eq!(Workspace::load(&path), Some(ws));

        std::fs::write(&path, "{ estragado").unwrap();
        assert_eq!(Workspace::load(&path), None);
    }

    #[test]
    fn comando_de_cada_painel() {
        let dir = tempfile::tempdir().unwrap();
        let pasta = dir.path().to_path_buf();
        let shell = || CommandBuilder::new("cmd.exe");
        let argv = |cmd: &CommandBuilder| -> Vec<String> {
            cmd.get_argv()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };

        let claude = SavedPane {
            cwd: Some(pasta.clone()),
            claude_session: Some("0123abcd-0000".into()),
        };
        let resume = sessions::resume_in;
        let cmd = command_for(&claude, &shell, &resume);
        assert_eq!(
            argv(&cmd).last().unwrap(),
            &sessions::resume_line("0123abcd-0000")
        );
        assert_eq!(cmd.get_cwd().map(PathBuf::from), Some(pasta.clone()));

        // Pasta que não existe mais: shell na pasta padrão, sem retomar.
        let sumiu = SavedPane {
            cwd: Some(pasta.join("sumiu")),
            claude_session: Some("0123abcd-0000".into()),
        };
        let cmd = command_for(&sumiu, &shell, &resume);
        assert_eq!(argv(&cmd), ["cmd.exe"]);
        assert_eq!(cmd.get_cwd(), None);

        // Id estranho não vira linha de comando.
        let estranho = SavedPane {
            cwd: Some(pasta.clone()),
            claude_session: Some("x; rm -rf".into()),
        };
        let cmd = command_for(&estranho, &shell, &resume);
        assert_eq!(argv(&cmd), ["cmd.exe"]);
        assert_eq!(cmd.get_cwd().map(PathBuf::from), Some(pasta));
    }

    #[test]
    fn processo_vivo() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(u32::MAX - 1));
    }
}
