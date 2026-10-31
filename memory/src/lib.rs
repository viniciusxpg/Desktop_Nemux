//! Armazenamento da memória do Nemux: SQLite com busca de texto (FTS5).

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, params};

pub use rusqlite::Error;

/// Uma memória salva.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub project: String,
    pub content: String,
    pub created_at: String,
}

pub struct Memory {
    conn: Connection,
}

/// Uma sessão do Claude Code rodando num painel do Nemux (gravada pelo `nemux-mcp`).
#[derive(Debug, Clone, PartialEq)]
pub struct PaneSession {
    pub session_id: String,
    /// Pasta onde o Claude roda.
    pub cwd: Option<String>,
    /// Processo do `nemux-mcp` dessa sessão: enquanto ele existe, a sessão está aberta.
    pub pid: Option<u32>,
}

/// Um limite do plano do Claude: quanto foi usado e quando volta a zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limit {
    /// De 0 a 100 (pode passar de 100).
    pub used_pct: f64,
    /// Segundos Unix em que o limite volta a zero.
    pub resets_at: i64,
}

/// Uso do Claude Code informado pela barra de status de uma sessão (`nemux-mcp statusline`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub session_id: String,
    /// Painel do Nemux onde a sessão roda (`NEMUX_PANE`), se roda num.
    pub pane: Option<String>,
    /// Processo do Claude: enquanto ele existe, a sessão está aberta.
    pub pid: Option<u32>,
    /// Quanto do contexto a sessão ocupa, de 0 a 100.
    pub context_pct: Option<f64>,
    pub five_hour: Option<Limit>,
    pub seven_day: Option<Limit>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS memories (
    id         INTEGER PRIMARY KEY,
    project    TEXT NOT NULL,
    content    TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    content, content='memories', content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;

-- Sessões do Claude abertas em cada painel do Nemux. Um painel pode ter várias
-- (um Claude que roda outro), e a mais antiga é a principal.
CREATE TABLE IF NOT EXISTS claude_panes (
    id         INTEGER PRIMARY KEY,
    pane       TEXT NOT NULL,
    session_id TEXT NOT NULL,
    cwd        TEXT,
    pid        INTEGER,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (pane, session_id)
);

-- Uso de cada sessão do Claude, gravado pela barra de status (nemux-mcp statusline).
CREATE TABLE IF NOT EXISTS claude_usage (
    session_id       TEXT PRIMARY KEY,
    pane             TEXT,
    pid              INTEGER,
    context_pct      REAL,
    five_hour_pct    REAL,
    five_hour_resets INTEGER,
    seven_day_pct    REAL,
    seven_day_resets INTEGER,
    updated_at       INTEGER NOT NULL
);
";

/// Segundos Unix, para `Memory::save_usage` e `Memory::limits`.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Caminho padrão do banco: `%APPDATA%\nemux\nemux.db` no Windows.
/// Pode ser trocado pela variável de ambiente `NEMUX_DB`.
pub fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("NEMUX_DB") {
        return path.into();
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nemux")
        .join("nemux.db")
}

impl Memory {
    /// Abre (ou cria) o banco no caminho informado.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            // Se falhar, o próprio Connection::open devolve o erro.
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    /// Banco só em memória, útil para testes.
    pub fn open_in_memory() -> Result<Self, Error> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, Error> {
        // Várias sessões do Claude (e o app) usam o mesmo arquivo: WAL deixa ler
        // enquanto outro grava, e o busy_timeout espera em vez de falhar.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn save(&self, project: &str, content: &str) -> Result<i64, Error> {
        self.conn.execute(
            "INSERT INTO memories (project, content) VALUES (?1, ?2)",
            params![project, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete(&self, id: i64) -> Result<bool, Error> {
        Ok(self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])?
            > 0)
    }

    /// Busca por texto livre, mais relevantes primeiro. Cada palavra é procurada
    /// separadamente, então caracteres especiais do FTS5 não causam erro.
    pub fn search(
        &self,
        text: &str,
        project: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Entry>, Error> {
        let Some(query) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.project, m.content, datetime(m.created_at, 'localtime')
             FROM memories_fts f JOIN memories m ON m.id = f.rowid
             WHERE memories_fts MATCH ?1 AND (?2 IS NULL OR m.project = ?2)
             ORDER BY rank LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![query, project, limit as i64], row_to_entry)?;
        rows.collect()
    }

    /// Memórias mais recentes, de um projeto ou de todos (`None`).
    pub fn recent(&self, project: Option<&str>, limit: usize) -> Result<Vec<Entry>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project, content, datetime(created_at, 'localtime') FROM memories
             WHERE ?1 IS NULL OR project = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![project, limit as i64], row_to_entry)?;
        rows.collect()
    }

    /// Projetos com a quantidade de memórias, os usados mais recentemente primeiro.
    pub fn projects(&self) -> Result<Vec<(String, usize)>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT project, COUNT(*) FROM memories GROUP BY project ORDER BY MAX(id) DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as usize)))?;
        rows.collect()
    }

    /// Registra que a sessão `session.session_id` do Claude está rodando no painel `pane`.
    /// Também apaga registros com mais de uma semana, de sessões que não se desregistraram.
    pub fn add_pane_session(&self, pane: &str, session: &PaneSession) -> Result<(), Error> {
        self.conn.execute(
            "DELETE FROM claude_panes WHERE started_at < datetime('now', '-7 days')",
            [],
        )?;
        self.conn.execute(
            "INSERT INTO claude_panes (pane, session_id, cwd, pid) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (pane, session_id) DO UPDATE SET cwd = excluded.cwd, pid = excluded.pid",
            params![pane, session.session_id, session.cwd, session.pid],
        )?;
        Ok(())
    }

    pub fn remove_pane_session(&self, pane: &str, session_id: &str) -> Result<(), Error> {
        self.conn.execute(
            "DELETE FROM claude_panes WHERE pane = ?1 AND session_id = ?2",
            params![pane, session_id],
        )?;
        Ok(())
    }

    /// Sessões registradas no painel, da mais antiga para a mais nova.
    pub fn pane_sessions(&self, pane: &str) -> Result<Vec<PaneSession>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, cwd, pid FROM claude_panes WHERE pane = ?1 ORDER BY id")?;
        let rows = stmt.query_map([pane], |row| {
            Ok(PaneSession {
                session_id: row.get(0)?,
                cwd: row.get(1)?,
                pid: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Grava o uso de uma sessão. Os limites que a sessão não informou desta vez
    /// ficam como estavam (eles só vêm depois da primeira resposta). Não grava nada
    /// se nada mudou: a barra de status roda a cada atualização da tela do Claude, e
    /// cada gravação faz o app (e o painel de memória) recarregar.
    pub fn save_usage(&self, usage: &Usage, now: i64) -> Result<(), Error> {
        let old = self.usage(&usage.session_id)?;
        let new = Usage {
            five_hour: usage.five_hour.or(old.as_ref().and_then(|o| o.five_hour)),
            seven_day: usage.seven_day.or(old.as_ref().and_then(|o| o.seven_day)),
            ..usage.clone()
        };
        if old.as_ref() == Some(&new) {
            return Ok(());
        }
        self.conn.execute(
            "DELETE FROM claude_usage WHERE updated_at < ?1",
            [now - 8 * 86_400],
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO claude_usage (session_id, pane, pid, context_pct,
                 five_hour_pct, five_hour_resets, seven_day_pct, seven_day_resets, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                new.session_id,
                new.pane,
                new.pid,
                new.context_pct,
                new.five_hour.map(|l| l.used_pct),
                new.five_hour.map(|l| l.resets_at),
                new.seven_day.map(|l| l.used_pct),
                new.seven_day.map(|l| l.resets_at),
                now,
            ],
        )?;
        Ok(())
    }

    fn usage(&self, session_id: &str) -> Result<Option<Usage>, Error> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {USAGE_COLUMNS} FROM claude_usage WHERE session_id = ?1"
        ))?;
        let mut rows = stmt.query_map([session_id], row_to_usage)?;
        rows.next().transpose()
    }

    /// Uso mais recente de cada painel, os atualizados por último primeiro.
    pub fn pane_usage(&self) -> Result<Vec<Usage>, Error> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {USAGE_COLUMNS} FROM claude_usage WHERE pane IS NOT NULL
             ORDER BY updated_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map([], row_to_usage)?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for usage in rows {
            let usage = usage?;
            if seen.insert(usage.pane.clone()) {
                out.push(usage);
            }
        }
        Ok(out)
    }

    /// Limites de 5 horas e de 7 dias ainda valendo em `now`. Eles são da conta, não
    /// da sessão: entre as sessões, vale a janela mais nova e, nela, o maior uso
    /// (sessões paradas guardam o número de quando falaram por último).
    pub fn limits(&self, now: i64) -> Result<(Option<Limit>, Option<Limit>), Error> {
        let limit = |name: &str| -> Result<Option<Limit>, Error> {
            // Sessões diferentes podem ver o mesmo reinício com segundos de diferença.
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {name}_pct, {name}_resets FROM claude_usage
                 WHERE {name}_resets > ?1 AND {name}_pct IS NOT NULL
                 ORDER BY {name}_resets / 300 DESC, {name}_pct DESC LIMIT 1"
            ))?;
            let mut rows = stmt.query_map([now], |row| {
                Ok(Limit {
                    used_pct: row.get(0)?,
                    resets_at: row.get(1)?,
                })
            })?;
            rows.next().transpose()
        };
        Ok((limit("five_hour")?, limit("seven_day")?))
    }

    /// Muda sempre que outra conexão grava no banco; serve para saber quando recarregar.
    pub fn data_version(&self) -> Result<i64, Error> {
        self.conn
            .query_row("PRAGMA data_version", [], |row| row.get(0))
    }
}

/// Transforma texto livre em consulta FTS5: `"palavra1" OR "palavra2"`.
fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

const USAGE_COLUMNS: &str = "session_id, pane, pid, context_pct, five_hour_pct, \
     five_hour_resets, seven_day_pct, seven_day_resets";

fn row_to_usage(row: &rusqlite::Row) -> rusqlite::Result<Usage> {
    let limit = |pct: Option<f64>, resets: Option<i64>| {
        Some(Limit {
            used_pct: pct?,
            resets_at: resets?,
        })
    };
    Ok(Usage {
        session_id: row.get(0)?,
        pane: row.get(1)?,
        pid: row.get(2)?,
        context_pct: row.get(3)?,
        five_hour: limit(row.get(4)?, row.get(5)?),
        seven_day: limit(row.get(6)?, row.get(7)?),
    })
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        project: row.get(1)?,
        content: row.get(2)?,
        created_at: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_sessions_in_order() {
        let mem = Memory::open_in_memory().unwrap();
        let session = |id: &str| PaneSession {
            session_id: id.into(),
            cwd: Some("C:/proj".into()),
            pid: Some(42),
        };
        mem.add_pane_session("p1", &session("a")).unwrap();
        mem.add_pane_session("p1", &session("b")).unwrap();
        mem.add_pane_session("p2", &session("c")).unwrap();
        // Registrar de novo (o MCP reconectado) não duplica nem muda a ordem.
        mem.add_pane_session("p1", &session("a")).unwrap();
        let ids = |pane| -> Vec<String> {
            mem.pane_sessions(pane)
                .unwrap()
                .into_iter()
                .map(|s| s.session_id)
                .collect()
        };
        assert_eq!(ids("p1"), ["a", "b"]);
        assert_eq!(mem.pane_sessions("p2").unwrap(), [session("c")]);

        mem.remove_pane_session("p1", "a").unwrap();
        mem.remove_pane_session("p1", "c").unwrap();
        assert_eq!(ids("p1"), ["b"]);
        assert!(ids("nenhum").is_empty());
    }

    fn limit(used_pct: f64, resets_at: i64) -> Option<Limit> {
        Some(Limit {
            used_pct,
            resets_at,
        })
    }

    #[test]
    fn usage_keeps_limits_and_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nemux.db");
        let reader = Memory::open(&path).unwrap();
        let writer = Memory::open(&path).unwrap();
        let mut usage = Usage {
            session_id: "s1".into(),
            pane: Some("p1".into()),
            pid: Some(10),
            context_pct: Some(12.0),
            five_hour: limit(40.0, 2_000),
            seven_day: limit(10.0, 9_000),
        };
        writer.save_usage(&usage, 1_000).unwrap();

        // Sem limites desta vez (sessão nova, antes da primeira resposta): ficam os de antes.
        usage.five_hour = None;
        usage.seven_day = None;
        usage.context_pct = Some(30.0);
        writer.save_usage(&usage, 1_010).unwrap();
        let saved = reader.pane_usage().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].context_pct, Some(30.0));
        assert_eq!(saved[0].five_hour, limit(40.0, 2_000));

        // O mesmo de novo não grava: quem lê não precisa recarregar.
        let before = reader.data_version().unwrap();
        writer.save_usage(&usage, 1_020).unwrap();
        assert_eq!(reader.data_version().unwrap(), before);
    }

    #[test]
    fn limits_from_newest_window() {
        let mem = Memory::open_in_memory().unwrap();
        let session = |id: &str, five: Option<Limit>| Usage {
            session_id: id.into(),
            five_hour: five,
            seven_day: limit(20.0, 50_000),
            ..Usage::default()
        };
        // a: janela que já passou; b e c: a atual, vista com 2 s de diferença.
        mem.save_usage(&session("a", limit(99.0, 900)), 100)
            .unwrap();
        mem.save_usage(&session("b", limit(62.0, 5_000)), 100)
            .unwrap();
        mem.save_usage(&session("c", limit(55.0, 5_002)), 100)
            .unwrap();
        assert_eq!(
            mem.limits(1_000).unwrap(),
            (limit(62.0, 5_000), limit(20.0, 50_000))
        );
        assert_eq!(mem.limits(60_000).unwrap(), (None, None));
    }

    #[test]
    fn pane_usage_latest_per_pane() {
        let mem = Memory::open_in_memory().unwrap();
        let session = |id: &str, pane: Option<&str>, ctx: f64| Usage {
            session_id: id.into(),
            pane: pane.map(Into::into),
            context_pct: Some(ctx),
            ..Usage::default()
        };
        mem.save_usage(&session("velha", Some("p1"), 80.0), 100)
            .unwrap();
        mem.save_usage(&session("nova", Some("p1"), 5.0), 200)
            .unwrap();
        mem.save_usage(&session("outra", Some("p2"), 50.0), 150)
            .unwrap();
        mem.save_usage(&session("fora", None, 70.0), 300).unwrap();
        let ids: Vec<_> = mem
            .pane_usage()
            .unwrap()
            .into_iter()
            .map(|u| u.session_id)
            .collect();
        assert_eq!(ids, ["nova", "outra"]);

        // Registros com mais de 8 dias somem na próxima gravação.
        mem.save_usage(&session("depois", Some("p3"), 1.0), 200 + 9 * 86_400)
            .unwrap();
        assert_eq!(mem.pane_usage().unwrap().len(), 1);
    }

    #[test]
    fn save_and_search() {
        let mem = Memory::open_in_memory().unwrap();
        mem.save("nemux", "O usuário prefere respostas em português")
            .unwrap();
        mem.save("nemux", "A interface usa Slint").unwrap();

        let found = mem.search("slint", None, 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "A interface usa Slint");
    }

    #[test]
    fn search_free_text_and_project_filter() {
        let mem = Memory::open_in_memory().unwrap();
        mem.save("a", "usa portable-pty para terminais").unwrap();
        mem.save("b", "terminais no Linux").unwrap();

        // Hífen e aspas quebrariam uma consulta FTS5 crua.
        assert_eq!(mem.search("portable-pty \"x", None, 10).unwrap().len(), 1);
        assert_eq!(mem.search("terminais", None, 10).unwrap().len(), 2);
        assert_eq!(mem.search("terminais", Some("b"), 10).unwrap().len(), 1);
        assert!(mem.search("  --  ", None, 10).unwrap().is_empty());
        // Acentos são ignorados pelo tokenizador padrão.
        assert_eq!(
            mem.search("TERMINAIS linux", Some("b"), 10).unwrap().len(),
            1
        );
    }

    #[test]
    fn recent_and_projects() {
        let mem = Memory::open_in_memory().unwrap();
        mem.save("a", "um").unwrap();
        mem.save("b", "dois").unwrap();
        mem.save("a", "três").unwrap();

        let all: Vec<_> = mem
            .recent(None, 10)
            .unwrap()
            .into_iter()
            .map(|e| e.content)
            .collect();
        assert_eq!(all, ["três", "dois", "um"]);
        assert_eq!(mem.recent(Some("b"), 10).unwrap().len(), 1);
        assert_eq!(
            mem.projects().unwrap(),
            [("a".to_string(), 2), ("b".to_string(), 1)]
        );
    }

    #[test]
    fn data_version_sees_other_connections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nemux.db");
        let reader = Memory::open(&path).unwrap();
        let writer = Memory::open(&path).unwrap();

        let before = reader.data_version().unwrap();
        assert_eq!(reader.data_version().unwrap(), before);
        writer.save("p", "nova").unwrap();
        assert_ne!(reader.data_version().unwrap(), before);
        assert_eq!(reader.recent(None, 5).unwrap().len(), 1);
    }

    #[test]
    fn delete_removes_from_search() {
        let mem = Memory::open_in_memory().unwrap();
        let id = mem.save("nemux", "memória temporária").unwrap();
        assert!(mem.delete(id).unwrap());
        assert!(mem.search("temporária", None, 10).unwrap().is_empty());
    }

    #[test]
    fn persists_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nemux.db");
        Memory::open(&path)
            .unwrap()
            .save("p", "persistido")
            .unwrap();

        let recent = Memory::open(&path).unwrap().recent(Some("p"), 5).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].content, "persistido");
    }
}
