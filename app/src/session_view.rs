//! Lista "Retomar sessão do Claude": mostra as sessões salvas e diz como abrir a escolhida.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use portable_pty::CommandBuilder;
use slint::{ModelRc, VecModel};

use crate::sessions::{self, Session};
use crate::{AppWindow, SessionItem};

/// Quantas sessões (as mais recentes) a lista mostra.
const LIMIT: usize = 100;

/// Monta o comando do painel que retoma uma sessão; `None` se não der para retomar.
pub type CommandFor = Box<dyn Fn(&Session) -> Option<CommandBuilder>>;

pub struct SessionsView {
    /// Pasta das sessões do Claude Code (os testes usam uma temporária).
    pub dir: Option<PathBuf>,
    /// Comando do painel que retoma a sessão (os testes trocam o `claude` por um `cmd.exe`).
    pub command_for: CommandFor,
    all: Vec<Session>,
    /// O que está na tela, depois do filtro: os índices da interface apontam para cá.
    shown: Vec<Session>,
}

impl SessionsView {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            command_for: Box::new(sessions::resume_command),
            all: Vec::new(),
            shown: Vec::new(),
        }
    }

    /// Relê as sessões do disco e aplica o filtro.
    pub fn refresh(&mut self, ui: &AppWindow) {
        self.all = match &self.dir {
            Some(dir) => sessions::scan(dir, LIMIT),
            None => Vec::new(),
        };
        self.filter(ui);
    }

    /// Mostra só as sessões cujo título ou pasta contém o texto da busca.
    pub fn filter(&mut self, ui: &AppWindow) {
        let query = ui.get_sessions_query().to_lowercase();
        let query = query.trim();
        self.shown = self
            .all
            .iter()
            .filter(|s| {
                query.is_empty()
                    || s.title.to_lowercase().contains(query)
                    || s.cwd
                        .as_deref()
                        .is_some_and(|c| c.to_string_lossy().to_lowercase().contains(query))
            })
            .cloned()
            .collect();

        let now = SystemTime::now();
        let home = dirs::home_dir();
        let items: Vec<SessionItem> = self
            .shown
            .iter()
            .map(|s| SessionItem {
                title: s.title.as_str().into(),
                folder: s
                    .cwd
                    .as_deref()
                    .map_or_else(
                        || "(pasta desconhecida)".into(),
                        |c| display_path(c, home.as_deref()),
                    )
                    .into(),
                when: sessions::relative_time(s.modified, now).into(),
                active: s.is_active(now),
                available: s.is_available(),
            })
            .collect();
        ui.set_sessions(ModelRc::new(VecModel::from(items)));
        ui.set_sessions_orphans(self.orphans(now).count() as i32);
        ui.set_sessions_status(self.status().into());
    }

    fn status(&self) -> String {
        if self.dir.as_deref().is_none_or(|d| !d.is_dir()) {
            return "Pasta de sessões do Claude Code não encontrada.".into();
        }
        format!(
            "{} sessões. Abre numa aba nova com claude --resume; se a sessão estiver \
             aberta em outra janela, feche a outra para não haver duas cópias da conversa.",
            self.all.len()
        )
    }

    /// Comando para retomar a sessão no índice `index` da lista mostrada.
    pub fn command(&self, index: usize) -> Option<CommandBuilder> {
        self.shown.get(index).and_then(|s| (self.command_for)(s))
    }

    /// Sessões cuja pasta não existe mais (não dá para retomá-las), menos as em uso.
    fn orphans(&self, now: SystemTime) -> impl Iterator<Item = &Session> {
        self.all
            .iter()
            .filter(move |s| !s.is_available() && !s.is_active(now))
    }

    /// Apaga do disco a sessão no índice `index` da lista mostrada e relê a lista.
    /// Uma sessão em uso não é apagada: o Claude voltaria a gravar nela.
    pub fn delete(&mut self, ui: &AppWindow, index: usize) {
        let Some(session) = self.shown.get(index) else {
            return;
        };
        if session.is_active(SystemTime::now()) {
            ui.set_sessions_status(
                "Esta sessão está em uso: feche o Claude dela antes de excluí-la.".into(),
            );
            return;
        }
        let result = sessions::delete(session);
        let title = session.title.clone();
        self.refresh(ui);
        match result {
            Ok(()) => ui.set_sessions_status(format!("Sessão \"{title}\" excluída.").into()),
            Err(e) => ui.set_sessions_status(
                format!("Não foi possível excluir a sessão \"{title}\": {e}").into(),
            ),
        }
    }

    /// Apaga do disco todas as sessões cuja pasta não existe mais e relê a lista.
    pub fn delete_orphans(&mut self, ui: &AppWindow) {
        let (mut ok, mut failed) = (0, 0);
        for session in self.orphans(SystemTime::now()) {
            match sessions::delete(session) {
                Ok(()) => ok += 1,
                Err(e) => {
                    eprintln!(
                        "nemux: não foi possível excluir {}: {e}",
                        session.path.display()
                    );
                    failed += 1;
                }
            }
        }
        self.refresh(ui);
        let mut status = match ok {
            1 => "1 sessão sem pasta excluída.".to_string(),
            n => format!("{n} sessões sem pasta excluídas."),
        };
        if failed > 0 {
            status += &format!(" {failed} não puderam ser excluídas.");
        }
        ui.set_sessions_status(status.into());
    }
}

/// Caminho com a pasta do usuário trocada por `~`.
fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_becomes_tilde() {
        #[cfg(windows)]
        let (home, proj, fora) = ("C:\\Users\\Ana", "~\\proj", "D:\\x");
        #[cfg(not(windows))]
        let (home, proj, fora) = ("/home/ana", "~/proj", "/tmp/x");
        let home = Path::new(home);
        assert_eq!(display_path(&home.join("proj"), Some(home)), proj);
        assert_eq!(display_path(home, Some(home)), "~");
        assert_eq!(display_path(Path::new(fora), Some(home)), fora);
    }
}
