//! Uso do Claude Code: lê o que `nemux-mcp statusline` grava no banco (a barra de
//! status de cada sessão do Claude). Os limites do plano vão para a barra de abas;
//! o contexto de cada sessão, para o painel onde ela roda.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nemux_memory::{Limit, Memory};

use crate::UsageMeter;

/// Recarrega mesmo sem gravação nova: o tempo até zerar muda, e limites vencem.
const RELOAD: Duration = Duration::from_secs(30);

pub struct UsageView {
    path: PathBuf,
    memory: Option<Memory>,
    /// Último `data_version` visto, para recarregar quando uma sessão gravar.
    version: Option<i64>,
    loaded_at: Option<Instant>,
}

/// O que mostrar: medidores da barra de abas, o texto ao passar o mouse e o
/// contexto de cada painel (pelo `NEMUX_PANE` dele).
#[derive(Debug, PartialEq)]
pub struct Snapshot {
    pub meters: Vec<UsageMeter>,
    pub detail: String,
    pub contexts: HashMap<String, f64>,
}

impl UsageView {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            memory: None,
            version: None,
            loaded_at: None,
        }
    }

    /// Dados novos, se o banco mudou ou se faz tempo que não recarrega.
    pub fn poll(&mut self) -> Option<Snapshot> {
        if self.memory.is_none() {
            self.memory = Memory::open(&self.path).ok();
        }
        let memory = self.memory.as_ref()?;
        let version = memory.data_version().ok()?;
        let fresh = self.loaded_at.is_some_and(|t| t.elapsed() < RELOAD);
        if Some(version) == self.version && fresh {
            return None;
        }
        self.version = Some(version);
        self.loaded_at = Some(Instant::now());

        let now = nemux_memory::unix_now();
        let (five_hour, seven_day) = memory.limits(now).ok()?;
        // Só sessões abertas: a última gravação de um Claude que já fechou fica no banco.
        let contexts = memory
            .pane_usage()
            .ok()?
            .into_iter()
            .filter(|u| u.pid.is_some_and(crate::workspace::process_alive))
            .filter_map(|u| Some((u.pane?, u.context_pct?)))
            .collect();
        Some(snapshot(five_hour, seven_day, contexts, now))
    }
}

fn snapshot(
    five_hour: Option<Limit>,
    seven_day: Option<Limit>,
    contexts: HashMap<String, f64>,
    now: i64,
) -> Snapshot {
    let limits = [
        ("5h", "Limite de 5 horas", five_hour),
        ("7d", "Limite semanal", seven_day),
    ];
    let mut meters = Vec::new();
    let mut detail = Vec::new();
    for (label, name, limit) in limits {
        let Some(limit) = limit else { continue };
        meters.push(UsageMeter {
            label: label.into(),
            percent: limit.used_pct as f32,
        });
        detail.push(format!(
            "{name}: {}% usado, zera em {}",
            limit.used_pct.round(),
            duration(limit.resets_at - now)
        ));
    }
    Snapshot {
        meters,
        detail: detail.join("\n"),
        contexts,
    }
}

/// "menos de 1 min", "45 min", "2 h 13 min", "1 dia e 4 h", "3 dias".
fn duration(secs: i64) -> String {
    let (min, hours, days) = (secs / 60, secs / 3600, secs / 86_400);
    match secs {
        ..60 => "menos de 1 min".into(),
        60..3600 => format!("{min} min"),
        3600..86_400 if min % 60 == 0 => format!("{hours} h"),
        3600..86_400 => format!("{hours} h {} min", min % 60),
        _ => {
            let days_text = if days == 1 {
                "1 dia".to_string()
            } else {
                format!("{days} dias")
            };
            match hours % 24 {
                0 => days_text,
                h => format!("{days_text} e {h} h"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nemux_memory::Usage;

    #[test]
    fn durations() {
        assert_eq!(duration(-5), "menos de 1 min");
        assert_eq!(duration(59), "menos de 1 min");
        assert_eq!(duration(45 * 60 + 10), "45 min");
        assert_eq!(duration(2 * 3600), "2 h");
        assert_eq!(duration(2 * 3600 + 13 * 60), "2 h 13 min");
        assert_eq!(duration(86_400 + 4 * 3600), "1 dia e 4 h");
        assert_eq!(duration(3 * 86_400 + 59), "3 dias");
    }

    #[test]
    fn meters_and_detail() {
        let limit = |used_pct, resets_at| Limit {
            used_pct,
            resets_at,
        };
        let s = snapshot(
            Some(limit(61.6, 1_000 + 2 * 3600 + 13 * 60)),
            Some(limit(18.0, 1_000 + 3 * 86_400)),
            HashMap::new(),
            1_000,
        );
        let labels: Vec<_> = s.meters.iter().map(|m| m.label.to_string()).collect();
        assert_eq!(labels, ["5h", "7d"]);
        assert_eq!(s.meters[0].percent, 61.6);
        assert_eq!(
            s.detail,
            "Limite de 5 horas: 62% usado, zera em 2 h 13 min\n\
             Limite semanal: 18% usado, zera em 3 dias"
        );

        // Sem limites (antes da primeira resposta, ou sem assinatura): nada na barra.
        let s = snapshot(None, None, HashMap::new(), 1_000);
        assert!(s.meters.is_empty());
        assert_eq!(s.detail, "");
    }

    #[test]
    fn poll_reloads_when_a_session_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nemux.db");
        let writer = Memory::open(&path).unwrap();
        let mut view = UsageView::new(path);
        assert_eq!(view.poll().unwrap().contexts, HashMap::new());
        assert_eq!(view.poll(), None);

        let session = |id: &str, pid: u32, ctx: f64| Usage {
            session_id: id.into(),
            pane: Some(format!("painel-{id}")),
            pid: Some(pid),
            context_pct: Some(ctx),
            ..Usage::default()
        };
        let now = nemux_memory::unix_now();
        // Este processo está vivo; o PID u32::MAX não existe (Claude que já fechou).
        writer
            .save_usage(&session("a", std::process::id(), 41.0), now)
            .unwrap();
        writer
            .save_usage(&session("b", u32::MAX, 90.0), now)
            .unwrap();
        let contexts = view.poll().unwrap().contexts;
        assert_eq!(contexts, HashMap::from([("painel-a".to_string(), 41.0)]));
    }
}
