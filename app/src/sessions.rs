//! Sessões do Claude Code salvas em disco (`~/.claude/projects/<pasta>/<id>.jsonl`),
//! para retomá-las num painel do Nemux com `claude --resume <id>`.
//!
//! Os arquivos chegam a vários MB, então só o começo (pasta e primeira mensagem) e o
//! fim (título mais recente) de cada um são lidos.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use portable_pty::CommandBuilder;
use serde_json::Value;

const HEAD_BYTES: u64 = 64 * 1024;
const TAIL_BYTES: u64 = 256 * 1024;
const TITLE_CHARS: usize = 120;
/// Sessão gravada há menos que isso provavelmente está aberta em outra janela.
const ACTIVE_WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: String,
    /// Pasta onde a sessão rodou; `claude --resume` precisa ser chamado nela.
    pub cwd: Option<PathBuf>,
    pub title: String,
    pub modified: SystemTime,
}

impl Session {
    /// Gravada há pouco: provavelmente aberta em outro terminal.
    pub fn is_active(&self, now: SystemTime) -> bool {
        now.duration_since(self.modified)
            .is_ok_and(|d| d < ACTIVE_WINDOW)
    }

    /// A pasta ainda existe (sem ela, `claude --resume` não acha a sessão).
    pub fn is_available(&self) -> bool {
        self.cwd.as_deref().is_some_and(Path::is_dir)
    }
}

/// Pasta onde o Claude Code guarda as sessões: `CLAUDE_CONFIG_DIR` ou `~/.claude`.
pub fn projects_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => dirs::home_dir()?.join(".claude"),
    };
    Some(base.join("projects"))
}

/// As `limit` sessões mais recentes, da mais nova para a mais antiga.
pub fn scan(dir: &Path, limit: usize) -> Vec<Session> {
    let mut files: Vec<(PathBuf, SystemTime)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .flat_map(|project| {
            std::fs::read_dir(project.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    files
        .into_iter()
        .filter_map(|(path, modified)| read_session(&path, modified))
        .take(limit)
        .collect()
}

fn read_session(path: &Path, modified: SystemTime) -> Option<Session> {
    let id = path.file_stem()?.to_str()?.to_string();
    if !valid_id(&id) {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();

    let mut cwd = None;
    let mut first_prompt = None;
    for value in json_lines(&read_range(&mut file, 0, HEAD_BYTES.min(len))?, false) {
        if cwd.is_none() {
            cwd = value.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        }
        if first_prompt.is_none() {
            first_prompt = user_prompt(&value);
        }
        if cwd.is_some() && first_prompt.is_some() {
            break;
        }
    }

    let tail_start = len.saturating_sub(TAIL_BYTES);
    let tail = read_range(&mut file, tail_start, len - tail_start)?;
    let mut ai_title = None;
    let mut last_prompt = None;
    // Só interpreta como JSON as linhas que podem ter título: o fim do arquivo é grande.
    let wanted =
        |line: &[u8]| contains(line, b"\"ai-title\"") || contains(line, b"\"last-prompt\"");
    for value in json_lines_where(&tail, tail_start > 0, wanted) {
        match value.get("type").and_then(Value::as_str) {
            Some("ai-title") => ai_title = string_field(&value, "aiTitle"),
            Some("last-prompt") => last_prompt = string_field(&value, "lastPrompt"),
            _ => {}
        }
    }

    // Sessão sem nenhuma mensagem (aberta e fechada): não há o que retomar.
    let title = ai_title.or(first_prompt).or(last_prompt)?;
    Some(Session {
        id,
        cwd,
        title: one_line(&title),
        modified,
    })
}

fn read_range(file: &mut File, start: u64, len: u64) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity(len as usize);
    file.take(len).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Objetos JSON de cada linha completa. `skip_first` descarta a primeira linha,
/// que pode estar cortada ao meio quando a leitura começa no meio do arquivo.
fn json_lines(bytes: &[u8], skip_first: bool) -> impl Iterator<Item = Value> + '_ {
    json_lines_where(bytes, skip_first, |_| true)
}

fn json_lines_where<'a>(
    bytes: &'a [u8],
    skip_first: bool,
    wanted: impl Fn(&[u8]) -> bool + 'a,
) -> impl Iterator<Item = Value> + 'a {
    bytes
        .split(|&b| b == b'\n')
        .skip(skip_first as usize)
        .filter(move |line| wanted(line))
        .filter_map(|line| serde_json::from_slice(line).ok())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(haystack, needle).is_some()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    let s = value.get(key)?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Texto digitado pelo usuário numa entrada `user` (ignora comandos e mensagens internas).
fn user_prompt(value: &Value) -> Option<String> {
    if value.get("type")?.as_str()? != "user"
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let content = value.get("message")?.get("content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text")?.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let text = text.trim();
    let internal = text.starts_with('<') || text.starts_with("Caveat:");
    (!text.is_empty() && !internal).then(|| text.to_string())
}

/// Uma linha só, com no máximo `TITLE_CHARS` caracteres.
fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match joined.char_indices().nth(TITLE_CHARS) {
        Some((cut, _)) => format!("{}…", joined[..cut].trim_end()),
        None => joined,
    }
}

/// Ids são UUIDs; qualquer outra coisa não vai para a linha de comando.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Painel que retoma a sessão: um PowerShell na pasta dela rodando `claude --resume`,
/// que continua aberto quando o Claude termina.
pub fn resume_command(session: &Session) -> Option<CommandBuilder> {
    if !session.is_available() {
        return None;
    }
    resume_in(&session.id, session.cwd.as_ref()?)
}

/// `claude --resume <id>` num PowerShell na pasta `cwd`; `None` se o id não parece um id.
pub fn resume_in(id: &str, cwd: &Path) -> Option<CommandBuilder> {
    if !valid_id(id) {
        return None;
    }
    let mut cmd = CommandBuilder::new("powershell.exe");
    cmd.args([
        "-NoLogo",
        "-NoExit",
        "-Command",
        &format!("claude --resume {id}"),
    ]);
    cmd.cwd(cwd);
    cmd.env("TERM_PROGRAM", "nemux");
    Some(cmd)
}

/// "agora", "há 5 min", "há 3 h", "ontem", "há 4 dias".
pub fn relative_time(modified: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(modified).map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "agora".into(),
        60..3600 => format!("há {} min", secs / 60),
        3600..86_400 => format!("há {} h", secs / 3600),
        86_400..172_800 => "ontem".into(),
        _ => format!("há {} dias", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID_A: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
    const ID_B: &str = "22222222-aaaa-4bbb-8ccc-000000000002";

    fn line(v: serde_json::Value) -> String {
        format!("{v}\n")
    }

    fn write(dir: &Path, project: &str, id: &str, content: &str) -> PathBuf {
        let folder = dir.join(project);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{id}.jsonl"));
        std::fs::write(&path, content).unwrap();
        path
    }

    fn set_age(path: &Path, secs_ago: u64) {
        let t = SystemTime::now() - Duration::from_secs(secs_ago);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn reads_title_folder_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("meu-projeto");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd_s = cwd.to_str().unwrap();

        let a = write(
            dir.path(),
            "proj-a",
            ID_A,
            &[
                line(serde_json::json!({"type": "user", "cwd": cwd_s, "isMeta": true, "message": {"content": "<command-name>/clear</command-name>"}})),
                line(serde_json::json!({"type": "user", "cwd": cwd_s, "message": {"content": "  arrume o\nbuild  "}})),
                line(serde_json::json!({"type": "ai-title", "aiTitle": "Título antigo"})),
                line(serde_json::json!({"type": "ai-title", "aiTitle": "Corrigir o build"})),
            ]
            .concat(),
        );
        let b = write(
            dir.path(),
            "proj-b",
            ID_B,
            &line(
                serde_json::json!({"type": "user", "cwd": "C:/nao/existe", "message": {"content": [{"type": "text", "text": "só a primeira mensagem"}]}}),
            ),
        );
        set_age(&a, 3600);
        set_age(&b, 10);

        let sessions = scan(dir.path(), 10);
        let ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, [ID_B, ID_A], "a mais recente primeiro");

        let (b, a) = (&sessions[0], &sessions[1]);
        assert_eq!(a.title, "Corrigir o build", "o ai-title mais recente");
        assert_eq!(a.cwd.as_deref(), Some(cwd.as_path()));
        assert!(a.is_available());
        assert_eq!(b.title, "só a primeira mensagem");
        assert!(!b.is_available(), "a pasta não existe");

        let now = SystemTime::now();
        assert!(b.is_active(now));
        assert!(!a.is_active(now));
    }

    #[test]
    fn title_falls_back_to_first_prompt_and_ignores_internal_messages() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "p",
            ID_A,
            &[
                line(serde_json::json!({"type": "user", "cwd": "C:/x", "message": {"content": "Caveat: mensagem interna"}})),
                line(serde_json::json!({"type": "user", "cwd": "C:/x", "message": {"content": "pergunta de verdade"}})),
            ]
            .concat(),
        );
        assert_eq!(scan(dir.path(), 10)[0].title, "pergunta de verdade");
    }

    #[test]
    fn skips_empty_sessions_bad_names_and_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "p",
            ID_A,
            &line(serde_json::json!({"type": "mode", "cwd": "C:/x"})),
        );
        write(
            dir.path(),
            "p",
            "nao-e-um-id; calc",
            &line(serde_json::json!({"type": "ai-title", "aiTitle": "x"})),
        );
        // Subagentes ficam em subpastas da sessão: não aparecem.
        write(
            &dir.path().join("p"),
            ID_B,
            ID_B,
            &line(serde_json::json!({"type": "ai-title", "aiTitle": "sub"})),
        );
        assert!(scan(dir.path(), 10).is_empty());
    }

    #[test]
    fn title_from_tail_of_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let filler = line(
            serde_json::json!({"type": "assistant", "message": {"content": "x".repeat(1000)}}),
        );
        let content = [
            line(serde_json::json!({"type": "user", "cwd": "C:/x", "message": {"content": "início"}})),
            filler.repeat(600), // ~600 KB: o título fica além do começo lido
            line(serde_json::json!({"type": "ai-title", "aiTitle": "Título no fim"})),
        ]
        .concat();
        write(dir.path(), "p", ID_A, &content);
        assert_eq!(scan(dir.path(), 10)[0].title, "Título no fim");
    }

    #[test]
    fn long_titles_are_cut() {
        let t = one_line(&"palavra ".repeat(40));
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= TITLE_CHARS + 1);
    }

    #[test]
    fn resume_only_valid_and_available() {
        let dir = tempfile::tempdir().unwrap();
        let ok = Session {
            id: ID_A.into(),
            cwd: Some(dir.path().to_path_buf()),
            title: "t".into(),
            modified: SystemTime::now(),
        };
        let cmd = resume_command(&ok).unwrap();
        let argv: Vec<_> = cmd
            .get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(argv.last().unwrap(), &format!("claude --resume {ID_A}"));

        assert!(
            resume_command(&Session {
                cwd: Some("C:/nao/existe".into()),
                ..ok.clone()
            })
            .is_none()
        );
        assert!(
            resume_command(&Session {
                id: "x; rm -rf".into(),
                ..ok
            })
            .is_none()
        );
    }

    #[test]
    fn relative_times() {
        let now = SystemTime::now();
        let ago = |s: u64| relative_time(now - Duration::from_secs(s), now);
        assert_eq!(ago(5), "agora");
        assert_eq!(ago(300), "há 5 min");
        assert_eq!(ago(3 * 3600), "há 3 h");
        assert_eq!(ago(30 * 3600), "ontem");
        assert_eq!(ago(4 * 86_400), "há 4 dias");
    }
}
