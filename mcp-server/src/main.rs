//! Servidor MCP do Nemux: expõe a memória para as sessões do Claude via stdio.
//!
//! Uso: `nemux-mcp [caminho-do-banco]`. Sem argumento, usa `nemux_memory::default_path()`.
//! Atenção: o stdout é o canal do protocolo, então mensagens de log vão para o stderr.

use std::sync::{Arc, Mutex};

use nemux_memory::{Entry, Memory, PaneSession};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;

const DEFAULT_LIMIT: usize = 10;

#[derive(Deserialize, JsonSchema)]
struct SaveArgs {
    /// Nome do projeto a que a memória pertence (ex.: o nome da pasta do repositório).
    project: String,
    /// O fato a lembrar, escrito de forma curta e autoexplicativa.
    content: String,
}

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Palavras a procurar; memórias com mais palavras em comum aparecem primeiro.
    query: String,
    /// Limita a busca a um projeto. Omita para buscar em todos.
    project: Option<String>,
    /// Máximo de resultados (padrão 10).
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct RecentArgs {
    /// Nome do projeto.
    project: String,
    /// Máximo de resultados (padrão 10).
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteArgs {
    /// Id da memória, como aparece nos resultados (`#12` → 12).
    id: i64,
}

#[derive(Clone)]
struct NemuxServer {
    memory: Arc<Mutex<Memory>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl NemuxServer {
    fn new(memory: Memory) -> Self {
        Self {
            memory: Arc::new(Mutex::new(memory)),
            tool_router: Self::tool_router(),
        }
    }

    fn with_memory<T>(
        &self,
        f: impl FnOnce(&Memory) -> Result<T, nemux_memory::Error>,
    ) -> Result<T, String> {
        let memory = self
            .memory
            .lock()
            .map_err(|_| "banco de memória indisponível".to_string())?;
        f(&memory).map_err(|e| format!("erro no banco de memória: {e}"))
    }

    #[tool(
        description = "Salva um fato na memória persistente do Nemux, compartilhada entre sessões e terminais."
    )]
    fn memory_save(&self, Parameters(args): Parameters<SaveArgs>) -> Result<String, String> {
        let content = args.content.trim();
        if content.is_empty() {
            return Err("content está vazio".into());
        }
        let id = self.with_memory(|m| m.save(args.project.trim(), content))?;
        Ok(format!("Memória #{id} salva."))
    }

    #[tool(
        description = "Busca na memória do Nemux por palavras-chave. Use antes de começar uma tarefa para recuperar contexto de sessões anteriores."
    )]
    fn memory_search(&self, Parameters(args): Parameters<SearchArgs>) -> Result<String, String> {
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        let entries =
            self.with_memory(|m| m.search(&args.query, args.project.as_deref(), limit))?;
        Ok(format_entries(&entries))
    }

    #[tool(description = "Lista as memórias mais recentes de um projeto.")]
    fn memory_recent(&self, Parameters(args): Parameters<RecentArgs>) -> Result<String, String> {
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        let entries = self.with_memory(|m| m.recent(Some(&args.project), limit))?;
        Ok(format_entries(&entries))
    }

    #[tool(
        description = "Apaga uma memória pelo id. Use quando um fato salvo estiver errado ou desatualizado."
    )]
    fn memory_delete(&self, Parameters(args): Parameters<DeleteArgs>) -> Result<String, String> {
        if self.with_memory(|m| m.delete(args.id))? {
            Ok(format!("Memória #{} apagada.", args.id))
        } else {
            Err(format!("Memória #{} não existe.", args.id))
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for NemuxServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("nemux", env!("CARGO_PKG_VERSION")))
            .with_instructions(
            "Memória persistente do Nemux. Busque (memory_search) antes de tarefas para recuperar \
             contexto; salve (memory_save) decisões, preferências do usuário e fatos não óbvios \
             do projeto; apague (memory_delete) o que estiver errado.",
        )
    }
}

fn format_entries(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "Nenhuma memória encontrada.".into();
    }
    entries
        .iter()
        .map(|e| format!("#{} [{}] {}: {}", e.id, e.project, e.created_at, e.content))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or_else(nemux_memory::default_path);
    let memory = Memory::open(&path)?;
    eprintln!("nemux-mcp: usando {}", path.display());

    // Num painel do Nemux, diz ao app qual sessão do Claude roda ali, para ele
    // poder retomá-la ao reabrir. O registro some quando o Claude fecha.
    let pane = pane_session();
    if let Some((pane, session)) = &pane
        && let Err(e) = memory.add_pane_session(pane, session)
    {
        eprintln!("nemux-mcp: não foi possível registrar o painel: {e}");
    }

    let result = match NemuxServer::new(memory).serve(stdio()).await {
        Ok(service) => service.waiting().await.map(|_| ()).map_err(Into::into),
        Err(e) => Err(e.into()),
    };
    if let Some((pane, session)) = &pane
        && let Ok(memory) = Memory::open(&path)
    {
        let _ = memory.remove_pane_session(pane, &session.session_id);
    }
    result
}

/// Painel do Nemux (`NEMUX_PANE`) e sessão do Claude (`CLAUDE_CODE_SESSION_ID`, que o
/// Claude Code passa aos servidores MCP), se este servidor roda dentro de um.
fn pane_session() -> Option<(String, PaneSession)> {
    let pane = std::env::var("NEMUX_PANE").ok()?;
    let session_id = std::env::var("CLAUDE_CODE_SESSION_ID").ok()?;
    let cwd = std::env::current_dir()
        .ok()
        .map(|d| d.to_string_lossy().into_owned());
    Some((
        pane,
        PaneSession {
            session_id,
            cwd,
            // Este servidor vive enquanto a sessão estiver aberta.
            pid: Some(std::process::id()),
        },
    ))
}
