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
";

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
