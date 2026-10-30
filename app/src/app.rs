//! Estado do app: abas, painéis (cada um com seu terminal), seleção e área de transferência.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::CommandBuilder;
use slint::{Model, ModelRc, VecModel};

use crate::keys::{self, Mods};
use crate::layout::{self, Direction, Node, PaneId, Rect, Split};
use crate::search::{self, Match};
use crate::selection::Selection;
use crate::terminal::{Colors, Event, Rgb, Run, Terminal};
use crate::workspace::{self, Resume, SavedNode, Workspace};
use crate::{AppWindow, DividerView, PaneView, SelRect, TabInfo, TermLine, TermRun};

/// Espaço entre painéis vizinhos; nele aparece a cor de divisória do fundo.
pub const PANE_GAP: f32 = 2.0;
/// Saída nos primeiros instantes de um painel (o prompt, uma sessão retomada) não
/// conta como novidade numa aba escondida.
const QUIET_START: Duration = Duration::from_secs(3);
/// Menor painel que uma divisão pode criar.
const MIN_COLS: u16 = 20;
const MIN_ROWS: u16 = 4;

pub struct Pane {
    pub title: String,
    /// Identifica o painel fora do app (`NEMUX_PANE`): o `nemux-mcp` de um Claude
    /// aberto nele registra a sessão com esse nome.
    pub token: String,
    /// Pasta atual: a de quando abriu, até o shell informar outra (OSC 7 ou 9;9).
    pub cwd: Option<PathBuf>,
    /// Saída nova enquanto a aba estava escondida.
    pub activity: bool,
    /// O programa pediu atenção (sino ou notificação) e ninguém olhou ainda.
    pub attention: bool,
    pub(crate) started: Instant,
    pub term: Terminal,
    pub rows: u16,
    pub cols: u16,
    lines_model: Rc<VecModel<TermLine>>,
    /// Linhas mostradas na tela, para atualizar só as que mudaram.
    pub shown: Vec<Vec<Run>>,
}

/// Busca no histórico do painel ativo (Ctrl+Shift+F).
#[derive(Default)]
pub struct Search {
    pub query: String,
    /// Ocorrência escolhida e o painel onde ela foi achada.
    pub current: Option<(PaneId, Match)>,
    /// "3 de 17", "Nenhum resultado" ou vazio.
    pub status: String,
}

pub struct Tab {
    /// Nome dado pelo usuário; fica no lugar do título que o programa define.
    pub name: Option<String>,
    pub layout: Node,
    /// Painel que recebe o teclado.
    pub active: PaneId,
}

/// Medidas vindas da interface: área dos painéis e tamanho de uma célula (px lógicos).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub width: f32,
    pub height: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    /// Margem interna de cada painel.
    pub pad: f32,
    /// Largura reservada à direita de cada painel para a barra de rolagem.
    pub scrollbar: f32,
}

pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub panes: HashMap<PaneId, Pane>,
    next_id: PaneId,
    metrics: Metrics,
    events: mpsc::Sender<(PaneId, Event)>,
    /// Comando de cada novo painel (os testes trocam por `cmd.exe`).
    pub shell: Box<dyn Fn() -> CommandBuilder>,
    /// Comando que retoma uma sessão do Claude ao restaurar as abas (os testes trocam).
    pub resume: Box<Resume>,
    /// A janela do app está em primeiro plano? (Os testes trocam.)
    pub foreground: Box<dyn Fn() -> bool>,
    /// Chamado quando um painel pede atenção sem ninguém olhando: pisca a barra de tarefas.
    pub on_attention: Box<dyn Fn()>,
    /// Cores do terminal (da configuração).
    pub colors: Colors,
    clipboard: Option<arboard::Clipboard>,
    panes_model: Rc<VecModel<PaneView>>,
    /// Fixo entre quadros: recriar a lista entre o apertar e o soltar do botão perderia o clique.
    tabs_model: Rc<VecModel<TabInfo>>,
    /// Fixo durante o arraste: recriar a lista destruiria a divisória sendo arrastada.
    dividers_model: Rc<VecModel<DividerView>>,
    /// Seleção e o painel onde ela foi feita.
    pub selection: Option<(PaneId, Selection)>,
    /// O botão esquerdo está pressionado sobre um painel.
    selecting: bool,
    /// O ponteiro saiu da célula inicial (um clique simples não seleciona).
    dragged: bool,
    /// Texto capturado quando a seleção terminou; é o que será copiado.
    pub selected_text: Option<String>,
    /// Busca aberta; `None` quando a barra está fechada.
    pub search: Option<Search>,
}

impl App {
    pub fn new(
        events: mpsc::Sender<(PaneId, Event)>,
        metrics: Metrics,
        panes_model: Rc<VecModel<PaneView>>,
        tabs_model: Rc<VecModel<TabInfo>>,
        dividers_model: Rc<VecModel<DividerView>>,
    ) -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            panes: HashMap::new(),
            next_id: 0,
            metrics,
            events,
            shell: Box::new(|| shell_command(None)),
            resume: Box::new(crate::sessions::resume_in),
            foreground: Box::new(|| true),
            on_attention: Box::new(|| {}),
            colors: Colors::default(),
            clipboard: None,
            panes_model,
            tabs_model,
            dividers_model,
            selection: None,
            selecting: false,
            dragged: false,
            selected_text: None,
            search: None,
        }
    }

    fn area(&self) -> Rect {
        Rect::new(0.0, 0.0, self.metrics.width, self.metrics.height)
    }

    fn grid_size(&self, rect: Rect) -> (u16, u16) {
        let m = self.metrics;
        layout::grid_size(rect, m.cell_w, m.cell_h, m.pad, m.scrollbar)
    }

    pub fn active_pane_id(&self) -> Option<PaneId> {
        self.tabs.get(self.active).map(|t| t.active)
    }

    pub fn active_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.active_pane_id()?)
    }

    fn active_term(&self) -> Option<&Terminal> {
        self.active_pane().map(|p| &p.term)
    }

    fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.layout.leaves().contains(&pane))
    }

    /// Retângulos dos painéis da aba ativa, relativos à área dos painéis.
    pub fn active_rects(&self) -> Vec<(PaneId, Rect)> {
        self.tabs
            .get(self.active)
            .map(|t| t.layout.rects(self.area(), PANE_GAP))
            .unwrap_or_default()
    }

    fn spawn_pane(&mut self, mut cmd: CommandBuilder, rect: Rect) -> Option<PaneId> {
        let (cols, rows) = self.grid_size(rect);
        let id = self.next_id;
        self.next_id += 1;
        let token = pane_token(id);
        cmd.env("NEMUX_PANE", &token);
        let cwd = cmd.get_cwd().map(PathBuf::from);
        let events = self.events.clone();
        match Terminal::spawn(cmd, rows, cols, move |e| {
            let _ = events.send((id, e));
        }) {
            Ok(term) => {
                self.panes.insert(
                    id,
                    Pane {
                        title: format!("Terminal {}", id + 1),
                        token,
                        cwd,
                        activity: false,
                        attention: false,
                        started: Instant::now(),
                        term,
                        rows,
                        cols,
                        lines_model: Rc::new(VecModel::default()),
                        shown: Vec::new(),
                    },
                );
                Some(id)
            }
            Err(e) => {
                eprintln!("nemux: não foi possível abrir o terminal: {e}");
                None
            }
        }
    }

    pub fn new_tab(&mut self) {
        let cmd = (self.shell)();
        self.new_tab_with(cmd);
    }

    pub fn new_tab_with(&mut self, cmd: CommandBuilder) {
        if let Some(id) = self.spawn_pane(cmd, self.area()) {
            self.tabs.push(Tab {
                name: None,
                layout: Node::Leaf(id),
                active: id,
            });
            self.active = self.tabs.len() - 1;
        }
    }

    /// Reabre as abas salvas (cada painel na sua pasta, com o Claude retomado);
    /// devolve quantas abriram. Painéis que não abrem somem da árvore.
    pub fn restore(&mut self, saved: &Workspace) -> usize {
        for tab in &saved.tabs {
            let Some(layout) = self.restore_node(&tab.layout) else {
                continue;
            };
            let leaves = layout.leaves();
            let active = leaves.get(tab.active).unwrap_or(&leaves[0]);
            self.tabs.push(Tab {
                name: tab.name.clone(),
                active: *active,
                layout,
            });
        }
        self.active = saved.active_tab.min(self.tabs.len().saturating_sub(1));
        self.relayout();
        self.tabs.len()
    }

    fn restore_node(&mut self, node: &SavedNode) -> Option<Node> {
        match node {
            SavedNode::Pane(pane) => {
                let cmd = workspace::command_for(pane, &*self.shell, &*self.resume);
                self.spawn_pane(cmd, self.area()).map(Node::Leaf)
            }
            SavedNode::Split { down, ratio, a, b } => {
                match (self.restore_node(a), self.restore_node(b)) {
                    (Some(a), Some(b)) => Some(Node::Split {
                        split: if *down { Split::Down } else { Split::Right },
                        ratio: ratio.clamp(0.05, 0.95),
                        a: Box::new(a),
                        b: Box::new(b),
                    }),
                    (a, b) => a.or(b),
                }
            }
        }
    }

    /// Divide o painel ativo. Não divide se algum dos dois ficaria pequeno demais.
    pub fn split(&mut self, split: Split) -> bool {
        let cmd = (self.shell)();
        self.split_with(split, cmd)
    }

    pub fn split_with(&mut self, split: Split, cmd: CommandBuilder) -> bool {
        let Some(target) = self.active_pane_id() else {
            return false;
        };
        let Some((_, rect)) = self
            .active_rects()
            .into_iter()
            .find(|(id, _)| *id == target)
        else {
            return false;
        };
        let (a, b) = rect.halves(split, PANE_GAP);
        let fits = |r: Rect| {
            let (cols, rows) = self.grid_size(r);
            cols >= MIN_COLS && rows >= MIN_ROWS
        };
        if !fits(a) || !fits(b) {
            return false;
        }
        let Some(new) = self.spawn_pane(cmd, b) else {
            return false;
        };
        let tab = &mut self.tabs[self.active];
        tab.layout.split(target, split, new);
        tab.active = new;
        self.relayout();
        true
    }

    /// Fecha um painel; o vizinho ocupa o espaço. O último painel fecha a aba.
    pub fn close_pane(&mut self, pane: PaneId) {
        let Some(index) = self.tab_of(pane) else {
            return;
        };
        let tab = &mut self.tabs[index];
        if !tab.layout.remove(pane) {
            self.close_tab(index);
            return;
        }
        if tab.active == pane {
            tab.active = tab.layout.leaves()[0];
        }
        // O `Drop` do `Terminal` encerra o processo.
        self.panes.remove(&pane);
        self.relayout();
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        for id in tab.layout.leaves() {
            self.panes.remove(&id);
        }
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
    }

    /// Dá um nome à aba; um nome vazio volta ao título do programa.
    pub fn rename_tab(&mut self, index: usize, name: &str) {
        if let Some(tab) = self.tabs.get_mut(index) {
            let name = name.trim();
            tab.name = (!name.is_empty()).then(|| name.to_owned());
        }
    }

    /// Leva a aba `from` para a posição `to`; a aba ativa continua a mesma.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        let n = self.tabs.len();
        if from >= n {
            return;
        }
        let to = to.min(n - 1);
        if from == to {
            return;
        }
        let active = self.active;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = if active == from {
            to
        } else if from < active && active <= to {
            active - 1
        } else if to <= active && active < from {
            active + 1
        } else {
            active
        };
    }

    pub fn select_tab(&mut self, index: usize) {
        self.active = index.min(self.tabs.len().saturating_sub(1));
    }

    fn select_offset(&mut self, offset: isize) {
        let n = self.tabs.len() as isize;
        if n > 0 {
            self.active = (self.active as isize + offset).rem_euclid(n) as usize;
        }
    }

    /// Torna `pane` o painel ativo (e a aba dele a aba ativa).
    pub fn focus_pane(&mut self, pane: PaneId) {
        if let Some(index) = self.tab_of(pane) {
            self.active = index;
            self.tabs[index].active = pane;
        }
    }

    /// Move o foco para o painel vizinho. Devolve false se não há vizinho nessa direção.
    pub fn focus_direction(&mut self, dir: Direction) -> bool {
        let Some(from) = self.active_pane_id() else {
            return false;
        };
        match layout::neighbor(&self.active_rects(), from, dir) {
            Some(to) => {
                self.tabs[self.active].active = to;
                true
            }
            None => false,
        }
    }

    /// Arrasta a divisória `index` (na ordem de `Node::dividers`) até o ponto (`x`, `y`)
    /// da área dos painéis. Nenhum dos lados fica menor que o mínimo de uma divisão.
    pub fn drag_divider(&mut self, index: usize, x: f32, y: f32) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(d) = tab
            .layout
            .dividers(self.area(), PANE_GAP)
            .into_iter()
            .nth(index)
        else {
            return;
        };
        let m = self.metrics;
        let (pos, start, len, min) = match d.split {
            Split::Right => (
                x,
                d.area.x,
                d.area.w,
                MIN_COLS as f32 * m.cell_w + 2.0 * m.pad + m.scrollbar,
            ),
            Split::Down => (
                y,
                d.area.y,
                d.area.h,
                MIN_ROWS as f32 * m.cell_h + 2.0 * m.pad,
            ),
        };
        let space = len - PANE_GAP;
        if space <= 0.0 {
            return;
        }
        let ratio = (pos - start - PANE_GAP / 2.0) / space;
        let (lo, hi) = (min / space, 1.0 - min / space);
        let ratio = if lo < hi { ratio.clamp(lo, hi) } else { 0.5 };
        self.tabs[self.active].layout.set_ratio(&d.path, ratio);
        self.relayout();
    }

    /// Volta a divisória `index` para o meio.
    pub fn reset_divider(&mut self, index: usize) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if let Some(d) = tab
            .layout
            .dividers(Rect::default(), PANE_GAP)
            .into_iter()
            .nth(index)
        {
            tab.layout.set_ratio(&d.path, 0.5);
            self.relayout();
        }
    }

    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics != self.metrics {
            self.metrics = metrics;
            self.relayout();
        }
    }

    /// Ajusta o tamanho de cada terminal ao espaço do seu painel, em todas as abas.
    fn relayout(&mut self) {
        let (area, m) = (self.area(), self.metrics);
        for tab in &self.tabs {
            for (id, rect) in tab.layout.rects(area, PANE_GAP) {
                if let Some(pane) = self.panes.get_mut(&id) {
                    let (cols, rows) =
                        layout::grid_size(rect, m.cell_w, m.cell_h, m.pad, m.scrollbar);
                    pane.cols = cols;
                    pane.rows = rows;
                    pane.term.resize(rows, cols);
                }
            }
        }
    }

    /// O painel está na aba à vista?
    fn visible(&self, pane: PaneId) -> bool {
        self.tab_of(pane) == Some(self.active)
    }

    /// O usuário está olhando para o painel: à vista, com o foco e com a janela na frente.
    fn watched(&self, pane: PaneId) -> bool {
        self.active_pane_id() == Some(pane) && (self.foreground)()
    }

    pub fn on_event(&mut self, pane: PaneId, event: &Event) {
        match event {
            Event::Output => {
                let hidden = !self.visible(pane);
                if let Some(p) = self.panes.get_mut(&pane)
                    && hidden
                    && p.started.elapsed() >= QUIET_START
                {
                    p.activity = true;
                }
            }
            Event::Attention => {
                let watched = self.watched(pane);
                if let Some(p) = self.panes.get_mut(&pane)
                    && !watched
                {
                    p.attention = true;
                    (self.on_attention)();
                }
            }
            Event::Title(title) if !title.trim().is_empty() => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.title = title.clone();
                }
            }
            Event::Title(_) => {}
            Event::Cwd(cwd) => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.cwd = Some(cwd.clone());
                }
            }
            Event::Exited => self.close_pane(pane),
        }
    }

    /// Painel sob o ponto (`x`, `y`) da área dos painéis.
    pub fn pane_at(&self, x: f32, y: f32) -> Option<PaneId> {
        self.active_rects()
            .into_iter()
            .find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(id, _)| id)
    }

    /// Arquivos soltos na janela: cola os caminhos no painel `pane` (ou no ativo) e
    /// dá o foco a ele.
    pub fn drop_files(&mut self, pane: Option<PaneId>, paths: &[PathBuf]) {
        let Some(pane) = pane.or(self.active_pane_id()) else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        self.focus_pane(pane);
        if let Some(p) = self.panes.get(&pane) {
            p.term.scroll_to_bottom();
            p.term.paste(&dropped_paths(paths));
        }
    }

    pub fn scroll(&mut self, pane: PaneId, lines: isize) {
        if let Some(p) = self.panes.get(&pane) {
            p.term.scroll(lines);
        }
    }

    /// Roda do mouse sobre a célula (`row`, `col`) do painel; `lines` positivo é para cima.
    /// Como no Windows Terminal: se o programa ligou o mouse (Claude Code), a roda vai
    /// para ele; na tela alternativa sem mouse (`less`), vira setas; senão rola o histórico.
    pub fn wheel(&mut self, pane: PaneId, lines: i32, row: i32, col: i32) {
        let Some((row, col)) = self.clamp_cell(pane, row, col) else {
            return;
        };
        let Some(p) = self.panes.get(&pane) else {
            return;
        };
        let up = lines > 0;
        if let Some(encoding) = p.term.mouse_reporting() {
            p.term.write(&keys::encode_wheel(up, row, col, encoding));
        } else if p.term.alternate_screen() {
            let arrow = if up { "\u{F700}" } else { "\u{F701}" };
            if let Some(bytes) = keys::encode(arrow, Mods::default(), p.term.application_cursor()) {
                p.term.write(&bytes.repeat(lines.unsigned_abs() as usize));
            }
        } else {
            p.term.scroll(lines as isize);
        }
    }

    /// Posiciona o histórico do painel `offset` linhas acima do fim (barra de rolagem).
    pub fn scroll_to(&mut self, pane: PaneId, offset: usize) {
        if let Some(p) = self.panes.get(&pane) {
            p.term.scroll_to(offset);
        }
    }

    // --- Seleção e área de transferência ---

    /// Converte coordenadas vindas da interface (podem sair da grade) numa célula do painel.
    fn clamp_cell(&self, pane: PaneId, row: i32, col: i32) -> Option<(u16, u16)> {
        let p = self.panes.get(&pane)?;
        let row = row.clamp(0, p.rows.saturating_sub(1) as i32) as u16;
        let col = col.clamp(0, p.cols.saturating_sub(1) as i32) as u16;
        Some((row, col))
    }

    pub fn start_selection(&mut self, pane: PaneId, row: i32, col: i32) {
        self.focus_pane(pane);
        let Some((row, col)) = self.clamp_cell(pane, row, col) else {
            return;
        };
        self.selection = Some((pane, Selection::at(row, col)));
        self.selected_text = None;
        self.selecting = true;
        self.dragged = false;
    }

    pub fn move_selection(&mut self, row: i32, col: i32) {
        let Some((pane, _)) = self.selection else {
            return;
        };
        let Some(cell) = self.clamp_cell(pane, row, col) else {
            return;
        };
        if let (true, Some((_, sel))) = (self.selecting, self.selection.as_mut()) {
            sel.head = cell;
            self.dragged |= sel.head != sel.anchor;
        }
    }

    pub fn end_selection(&mut self) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        if self.dragged {
            self.capture_selection();
        } else {
            self.clear_selection();
        }
    }

    pub fn select_word(&mut self, pane: PaneId, row: i32, col: i32) {
        self.focus_pane(pane);
        let (Some((row, col)), Some(p)) = (self.clamp_cell(pane, row, col), self.panes.get(&pane))
        else {
            return;
        };
        let sel = p
            .term
            .with_screen(|screen| Selection::word(screen, row, col));
        self.selection = Some((pane, sel));
        self.selecting = false;
        self.capture_selection();
    }

    fn capture_selection(&mut self) {
        let text = match self.selection {
            Some((pane, sel)) => self
                .panes
                .get(&pane)
                .map(|p| p.term.with_screen(|s| sel.text(s)))
                .unwrap_or_default(),
            None => String::new(),
        };
        if text.trim().is_empty() {
            self.clear_selection();
        } else {
            self.selected_text = Some(text);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.selected_text = None;
        self.selecting = false;
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// Copia a seleção para a área de transferência. Devolve false se não havia seleção.
    fn copy(&mut self) -> bool {
        let Some(text) = self.selected_text.take() else {
            return false;
        };
        if let Some(clipboard) = self.clipboard()
            && let Err(e) = clipboard.set_text(text)
        {
            eprintln!("nemux: não foi possível copiar: {e}");
        }
        self.clear_selection();
        true
    }

    fn paste(&mut self) {
        let text = self.clipboard().and_then(|c| c.get_text().ok());
        if let (Some(text), Some(term)) = (text, self.active_term()) {
            term.scroll_to_bottom();
            term.paste(&text);
        }
    }

    /// Botão direito: copia se houver seleção, senão cola no painel clicado
    /// (como no Windows Terminal).
    pub fn secondary_click(&mut self, pane: PaneId) {
        self.focus_pane(pane);
        if !self.copy() {
            self.paste();
        }
    }

    // --- Busca ---

    /// O texto da busca mudou: escolhe a ocorrência mais perto do fim do que está à vista.
    pub fn set_search_query(&mut self, query: &str) {
        let search = self.search.get_or_insert_with(Search::default);
        search.query = query.to_owned();
        search.current = None;
        let Some(pane) = self.active_pane_id() else {
            return;
        };
        let (matches, bottom) = {
            let term = &self.panes[&pane].term;
            let rows = self.panes[&pane].rows as usize;
            (
                term.find_all(query),
                term.with_screen(search::top_line) + rows,
            )
        };
        let index = matches
            .iter()
            .rposition(|m| m.line < bottom)
            .or(matches.len().checked_sub(1));
        self.show_match(pane, &matches, index);
    }

    /// Vai para a ocorrência acima (`up`) ou abaixo da atual, dando a volta nas pontas.
    pub fn search_step(&mut self, up: bool) {
        let (Some(search), Some(pane)) = (&self.search, self.active_pane_id()) else {
            return;
        };
        let term = &self.panes[&pane].term;
        let matches = term.find_all(&search.query);
        // Sem ocorrência escolhida neste painel, parte da tela à vista.
        let from = match search.current {
            Some((p, m)) if p == pane => m,
            _ => {
                let top = term.with_screen(search::top_line);
                let line = if up {
                    top + self.panes[&pane].rows as usize
                } else {
                    top.saturating_sub(1)
                };
                Match {
                    line,
                    col: 0,
                    width: 0,
                }
            }
        };
        let index = if up {
            matches
                .iter()
                .rposition(|m| *m < from)
                .or(matches.len().checked_sub(1))
        } else {
            matches
                .iter()
                .position(|m| *m > from)
                .or((!matches.is_empty()).then_some(0))
        };
        self.show_match(pane, &matches, index);
    }

    fn show_match(&mut self, pane: PaneId, matches: &[Match], index: Option<usize>) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        search.current = index.map(|i| (pane, matches[i]));
        search.status = match index {
            _ if search.query.is_empty() => String::new(),
            Some(i) => format!("{} de {}", i + 1, matches.len()),
            None => "Nenhum resultado".to_owned(),
        };
        if let (Some((_, m)), Some(p)) = (search.current, self.panes.get(&pane)) {
            p.term.scroll_to_line(m.line);
        }
    }

    pub fn close_search(&mut self) {
        self.search = None;
    }

    // --- Teclado ---

    /// Trata uma tecla: atalhos do Nemux primeiro, senão envia ao painel ativo.
    /// Devolve true se a tecla foi consumida.
    pub fn handle_key(&mut self, text: &str, mods: Mods) -> bool {
        if self.handle_shortcut(text, mods) {
            return true;
        }
        let Some(term) = self.active_term() else {
            return false;
        };
        let Some(bytes) = keys::encode(text, mods, term.application_cursor()) else {
            return false;
        };
        term.scroll_to_bottom();
        term.write(&bytes);
        // Digitar descarta a seleção, como no Windows Terminal.
        self.clear_selection();
        true
    }

    fn handle_shortcut(&mut self, text: &str, mods: Mods) -> bool {
        let key = text.to_lowercase();
        let several_panes = self.active_rects().len() > 1;
        match (mods.ctrl, mods.shift, mods.alt, key.as_str()) {
            (true, true, false, "c") => {
                self.copy();
            }
            // Ctrl+C só copia se houver seleção; sem ela, vai para o programa (interromper).
            (true, false, false, "c") if self.selected_text.is_some() => {
                self.copy();
            }
            (true, true, false, "t") => self.new_tab(),
            (true, true, false, "w") => {
                if let Some(pane) = self.active_pane_id() {
                    self.close_pane(pane);
                }
            }
            (true, _, false, "v") => self.paste(),
            (true, false, false, "\t") => self.select_offset(1),
            (true, true, false, "\t") | (true, true, false, "\u{19}") => self.select_offset(-1),
            // Alt+Shift+= / Alt+Shift+-: dividir (como no Windows Terminal). Com Shift,
            // a tecla chega como "+" / "_" na maioria dos teclados.
            (false, true, true, "+" | "=") => {
                self.split(Split::Right);
            }
            (false, true, true, "_" | "-") => {
                self.split(Split::Down);
            }
            // Alt+setas movem o foco entre painéis; com um painel só, vão para o programa.
            (false, false, true, "\u{f700}" | "\u{f701}" | "\u{f702}" | "\u{f703}")
                if several_panes =>
            {
                let dir = match key.as_str() {
                    "\u{f700}" => Direction::Up,
                    "\u{f701}" => Direction::Down,
                    "\u{f702}" => Direction::Left,
                    _ => Direction::Right,
                };
                self.focus_direction(dir);
            }
            // Ctrl+Shift+PageUp/PageDown: leva a aba ativa para a esquerda / direita.
            (true, true, false, "\u{f72c}") => {
                self.move_tab(self.active, self.active.saturating_sub(1));
            }
            (true, true, false, "\u{f72d}") => self.move_tab(self.active, self.active + 1),
            // Shift+PageUp/PageDown: rola o histórico do painel ativo.
            (false, true, false, "\u{f72c}" | "\u{f72d}") => {
                if let Some(pane) = self.active_pane() {
                    let page = pane.rows.saturating_sub(1).max(1) as isize;
                    pane.term
                        .scroll(if key == "\u{f72c}" { page } else { -page });
                }
            }
            _ => return false,
        }
        true
    }

    // --- Desenho ---

    /// Apaga as marcas do que está à vista: a novidade da aba ativa e o pedido de
    /// atenção do painel com o foco (se a janela estiver na frente).
    fn clear_seen(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let watched = self.watched(tab.active).then_some(tab.active);
        for id in tab.layout.leaves() {
            if let Some(p) = self.panes.get_mut(&id) {
                p.activity = false;
                if Some(id) == watched {
                    p.attention = false;
                }
            }
        }
    }

    pub fn render(&mut self, ui: &AppWindow) {
        self.clear_seen();
        let tabs: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|t| {
                let panes: Vec<&Pane> = t
                    .layout
                    .leaves()
                    .iter()
                    .filter_map(|id| self.panes.get(id))
                    .collect();
                TabInfo {
                    title: t
                        .name
                        .as_deref()
                        .or_else(|| self.panes.get(&t.active).map(|p| p.title.as_str()))
                        .unwrap_or("")
                        .into(),
                    activity: panes.iter().any(|p| p.activity),
                    attention: panes.iter().any(|p| p.attention),
                }
            })
            .collect();
        update_model(&self.tabs_model, tabs);
        ui.set_active_tab(self.active as i32);

        // A seleção só vale enquanto o painel dela está visível.
        if let Some((pane, _)) = self.selection
            && self.tab_of(pane) != Some(self.active)
        {
            self.clear_selection();
        }

        let active = self.active_pane_id();
        let views: Vec<PaneView> = self
            .active_rects()
            .into_iter()
            .filter_map(|(id, rect)| self.pane_view(id, rect, Some(id) == active))
            .collect();

        let dividers: Vec<DividerView> = self
            .tabs
            .get(self.active)
            .map(|t| t.layout.dividers(self.area(), PANE_GAP))
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, d)| DividerView {
                index: i as i32,
                x: d.rect.x,
                y: d.rect.y,
                width: d.rect.w,
                height: d.rect.h,
                vertical: d.split == Split::Right,
            })
            .collect();
        update_model(&self.dividers_model, dividers);
        ui.set_search_status(
            self.search
                .as_ref()
                .map_or("", |s| s.status.as_str())
                .into(),
        );

        let model = &self.panes_model;
        let same_panes = model.row_count() == views.len()
            && model.iter().zip(&views).all(|(old, new)| old.id == new.id);
        if same_panes {
            for (row, view) in views.into_iter().enumerate() {
                model.set_row_data(row, view);
            }
        } else {
            model.set_vec(views);
        }
    }

    fn pane_view(&mut self, id: PaneId, rect: Rect, active: bool) -> Option<PaneView> {
        let selection = self.selection.filter(|(p, _)| *p == id).map(|(_, s)| s);
        // A busca vale para o painel ativo.
        let search = self
            .search
            .as_ref()
            .filter(|s| active && !s.query.is_empty());
        let pane = self.panes.get_mut(&id)?;
        let snap = pane.term.snapshot(&self.colors);
        let to_rect = |(row, col, width): (u16, u16, u16)| SelRect {
            row: row as i32,
            col: col as i32,
            width: width as i32,
        };
        let (matches, current_match) = match search {
            Some(s) => pane.term.with_screen(|screen| {
                let top = search::top_line(screen);
                let current = s
                    .current
                    .filter(|(p, m)| *p == id && m.line >= top && m.line - top < pane.rows as usize)
                    .map(|(_, m)| to_rect(((m.line - top) as u16, m.col, m.width)));
                (search::visible(screen, &s.query), current)
            }),
            None => (Vec::new(), None),
        };

        let model = &pane.lines_model;
        if model.row_count() != snap.lines.len() {
            model.set_vec(snap.lines.iter().map(|l| to_line(l)).collect::<Vec<_>>());
        } else {
            for (row, line) in snap.lines.iter().enumerate() {
                if pane.shown.get(row) != Some(line) {
                    model.set_row_data(row, to_line(line));
                }
            }
        }
        pane.shown = snap.lines;

        let (cursor_row, cursor_col) = snap.cursor.map_or((-1, 0), |(r, c)| (r as i32, c as i32));
        let sel_rects: Vec<SelRect> = selection
            .map(|s| s.rects(pane.cols))
            .unwrap_or_default()
            .into_iter()
            .map(to_rect)
            .collect();
        let matches: Vec<SelRect> = matches.into_iter().map(to_rect).collect();

        Some(PaneView {
            id: id as i32,
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
            lines: ModelRc::from(pane.lines_model.clone()),
            cursor_row,
            cursor_col,
            selection: ModelRc::new(VecModel::from(sel_rects)),
            matches: ModelRc::new(VecModel::from(matches)),
            current_match: current_match.unwrap_or(SelRect {
                row: -1,
                col: 0,
                width: 0,
            }),
            active,
            attention: pane.attention,
            rows: pane.rows as i32,
            history: snap.history as i32,
            offset: snap.offset as i32,
        })
    }
}

/// Caminhos separados por espaço; entre aspas os que o shell partiria ou interpretaria
/// (espaços e caracteres especiais do PowerShell e do cmd).
fn dropped_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| {
            let p = p.to_string_lossy();
            if p.chars()
                .any(|c| c.is_whitespace() || "&()[]{};,'`$@#^%!".contains(c))
            {
                format!("\"{p}\"")
            } else {
                p.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Único entre os painéis de todas as execuções do app: processo, painel e instante.
fn pane_token(id: PaneId) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{id}-{nanos:x}", std::process::id())
}

/// Troca o conteúdo de `model` por `rows` mexendo só nas linhas que mudaram, para a
/// interface não recriar os elementos (e perder um clique ou arraste em andamento).
fn update_model<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() == rows.len() {
        for (row, data) in rows.into_iter().enumerate() {
            if model.row_data(row).as_ref() != Some(&data) {
                model.set_row_data(row, data);
            }
        }
    } else {
        model.set_vec(rows);
    }
}

/// Shell de cada painel: `NEMUX_SHELL` (ex.: `cmd.exe`), o `shell` da configuração ou o
/// padrão do sistema (PowerShell no Windows, `$SHELL` nos outros), na pasta do usuário.
pub fn shell_command(configured: Option<&str>) -> CommandBuilder {
    let env = std::env::var("NEMUX_SHELL").ok();
    let default = default_shell();
    let shell = env.as_deref().or(configured).unwrap_or(&default);
    let mut parts = shell.split_whitespace();
    let mut cmd = CommandBuilder::new(parts.next().unwrap_or(&default));
    cmd.args(parts);
    if let Some(home) = dirs::home_dir() {
        cmd.cwd(home);
    }
    terminal_env(&mut cmd);
    cmd
}

#[cfg(windows)]
fn default_shell() -> String {
    "powershell.exe -NoLogo".to_owned()
}

#[cfg(not(windows))]
fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_owned())
}

/// Variáveis que dizem aos programas que terminal é este. Fora do Windows, `TERM`
/// escolhe as sequências que os programas usam; o parser (`vt100`) entende as do xterm.
pub fn terminal_env(cmd: &mut CommandBuilder) {
    cmd.env("TERM_PROGRAM", "nemux");
    if !cfg!(windows) {
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
    }
}

fn to_color((r, g, b): Rgb) -> slint::Color {
    slint::Color::from_rgb_u8(r, g, b)
}

pub fn to_line(runs: &[Run]) -> TermLine {
    let runs: Vec<TermRun> = runs
        .iter()
        .map(|r| TermRun {
            col: r.col as i32,
            width: r.width as i32,
            text: r.text.as_str().into(),
            fg: to_color(r.fg),
            bg: r.bg.map(to_color).unwrap_or_default(),
            has_bg: r.bg.is_some(),
            bold: r.bold,
            italic: r.italic,
            underline: r.underline,
        })
        .collect();
    TermLine {
        runs: ModelRc::new(VecModel::from(runs)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_paths_quote_only_when_needed() {
        let paths = [
            PathBuf::from(r"C:\src\main.rs"),
            PathBuf::from(r"C:\Meus Documentos\nota.txt"),
            PathBuf::from(r"C:\tmp\a&b.log"),
        ];
        assert_eq!(
            dropped_paths(&paths),
            r#"C:\src\main.rs "C:\Meus Documentos\nota.txt" "C:\tmp\a&b.log""#
        );
        assert_eq!(dropped_paths(&[]), "");
    }
}
