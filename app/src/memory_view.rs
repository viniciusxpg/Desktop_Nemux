//! Painel de memória: lê o mesmo banco que o servidor MCP usa (`nemux.db`).

use std::path::PathBuf;

use nemux_memory::{Entry, Memory};
use slint::{ModelRc, SharedString, VecModel};

use crate::{AppWindow, MemoryEntry};

/// Quantas memórias o painel mostra de uma vez.
const LIMIT: usize = 200;

pub struct MemoryView {
    path: PathBuf,
    /// Aberto só quando o painel é usado pela primeira vez.
    memory: Option<Memory>,
    /// Projetos na ordem do filtro (o índice 0 do filtro é "todos").
    projects: Vec<String>,
    /// Último `data_version` visto, para recarregar quando outra sessão gravar.
    version: Option<i64>,
}

impl MemoryView {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            memory: None,
            projects: Vec::new(),
            version: None,
        }
    }

    fn memory(&mut self) -> Result<&Memory, String> {
        if self.memory.is_none() {
            let memory = Memory::open(&self.path)
                .map_err(|e| format!("Não foi possível abrir {}: {e}", self.path.display()))?;
            self.memory = Some(memory);
        }
        Ok(self.memory.as_ref().expect("acabou de ser aberto"))
    }

    /// Recarrega projetos e memórias conforme a busca e o filtro do painel.
    pub fn refresh(&mut self, ui: &AppWindow) {
        if let Err(e) = self.load(ui) {
            ui.set_memory_entries(ModelRc::default());
            ui.set_memory_status(e.into());
        }
    }

    fn load(&mut self, ui: &AppWindow) -> Result<(), String> {
        // Guarda o projeto escolhido pelo nome, porque a ordem da lista pode mudar.
        let index = ui.get_memory_project_index();
        let selected = usize::try_from(index - 1)
            .ok()
            .and_then(|i| self.projects.get(i))
            .cloned();
        let query = ui.get_memory_query().to_string();
        let path = self.path.display().to_string();

        let memory = self.memory()?;
        let db_err = |e: nemux_memory::Error| format!("Erro no banco de memória: {e}");
        let projects = memory.projects().map_err(db_err)?;
        let project = selected.filter(|p| projects.iter().any(|(name, _)| name == p));
        let entries = if query.trim().is_empty() {
            memory.recent(project.as_deref(), LIMIT)
        } else {
            memory.search(&query, project.as_deref(), LIMIT)
        }
        .map_err(db_err)?;
        let version = memory.data_version().map_err(db_err)?;

        let total: usize = match &project {
            Some(p) => projects
                .iter()
                .find(|(name, _)| name == p)
                .map_or(0, |(_, n)| *n),
            None => projects.iter().map(|(_, n)| n).sum(),
        };
        let mut names: Vec<SharedString> = vec!["Todos os projetos".into()];
        names.extend(
            projects
                .iter()
                .map(|(name, n)| SharedString::from(format!("{name} ({n})"))),
        );
        let new_index = project
            .as_ref()
            .and_then(|p| projects.iter().position(|(name, _)| name == p))
            .map_or(0, |i| i as i32 + 1);

        self.projects = projects.into_iter().map(|(name, _)| name).collect();
        self.version = Some(version);

        ui.set_memory_projects(ModelRc::new(VecModel::from(names)));
        ui.set_memory_project_index(new_index);
        ui.set_memory_status(status(entries.len(), total, !query.trim().is_empty(), &path).into());
        let entries: Vec<MemoryEntry> = entries.into_iter().map(to_entry).collect();
        ui.set_memory_entries(ModelRc::new(VecModel::from(entries)));
        Ok(())
    }

    pub fn delete(&mut self, ui: &AppWindow, id: i32) {
        let result = self.memory().and_then(|m| {
            m.delete(id as i64)
                .map_err(|e| format!("Erro ao apagar a memória #{id}: {e}"))
        });
        match result {
            Ok(_) => self.refresh(ui),
            Err(e) => ui.set_memory_status(e.into()),
        }
    }

    /// Recarrega se outra conexão (ex.: o servidor MCP de uma sessão do Claude) gravou no banco.
    pub fn poll(&mut self, ui: &AppWindow) {
        let Some(memory) = &self.memory else { return };
        if let Ok(version) = memory.data_version()
            && Some(version) != self.version
        {
            self.refresh(ui);
        }
    }
}

fn to_entry(e: Entry) -> MemoryEntry {
    MemoryEntry {
        id: e.id as i32,
        project: e.project.into(),
        content: e.content.into(),
        created: e.created_at.into(),
    }
}

fn status(shown: usize, total: usize, searching: bool, path: &str) -> String {
    let count = match (searching, total) {
        (true, _) => format!("{shown} encontradas de {total}"),
        (false, 1) => "1 memória".to_string(),
        (false, n) if shown < n => format!("{shown} mais recentes de {n} memórias"),
        (false, n) => format!("{n} memórias"),
    };
    format!("{count} · {path}")
}
