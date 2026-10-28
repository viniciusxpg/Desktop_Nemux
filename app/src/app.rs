//! Estado do app: abas, painéis (cada um com seu terminal), seleção e área de transferência.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;

use portable_pty::CommandBuilder;
use slint::{Model, ModelRc, VecModel};

use crate::keys::{self, Mods};
use crate::layout::{self, Direction, Node, PaneId, Rect, Split};
use crate::selection::Selection;
use crate::terminal::{Event, Rgb, Run, Terminal};
use crate::{AppWindow, PaneView, SelRect, TabInfo, TermLine, TermRun};

/// Espaço entre painéis vizinhos; nele aparece a cor de divisória do fundo.
pub const PANE_GAP: f32 = 2.0;
/// Menor painel que uma divisão pode criar.
const MIN_COLS: u16 = 20;
const MIN_ROWS: u16 = 4;

pub struct Pane {
    pub title: String,
    pub term: Terminal,
    pub rows: u16,
    pub cols: u16,
    lines_model: Rc<VecModel<TermLine>>,
    /// Linhas mostradas na tela, para atualizar só as que mudaram.
    pub shown: Vec<Vec<Run>>,
}

pub struct Tab {
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
    clipboard: Option<arboard::Clipboard>,
    panes_model: Rc<VecModel<PaneView>>,
    /// Seleção e o painel onde ela foi feita.
    pub selection: Option<(PaneId, Selection)>,
    /// O botão esquerdo está pressionado sobre um painel.
    selecting: bool,
    /// O ponteiro saiu da célula inicial (um clique simples não seleciona).
    dragged: bool,
    /// Texto capturado quando a seleção terminou; é o que será copiado.
    pub selected_text: Option<String>,
}

impl App {
    pub fn new(
        events: mpsc::Sender<(PaneId, Event)>,
        metrics: Metrics,
        panes_model: Rc<VecModel<PaneView>>,
    ) -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            panes: HashMap::new(),
            next_id: 0,
            metrics,
            events,
            shell: Box::new(shell_command),
            clipboard: None,
            panes_model,
            selection: None,
            selecting: false,
            dragged: false,
            selected_text: None,
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

    fn spawn_pane(&mut self, cmd: CommandBuilder, rect: Rect) -> Option<PaneId> {
        let (cols, rows) = self.grid_size(rect);
        let id = self.next_id;
        self.next_id += 1;
        let events = self.events.clone();
        match Terminal::spawn(cmd, rows, cols, move |e| {
            let _ = events.send((id, e));
        }) {
            Ok(term) => {
                self.panes.insert(
                    id,
                    Pane {
                        title: format!("Terminal {}", id + 1),
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
                layout: Node::Leaf(id),
                active: id,
            });
            self.active = self.tabs.len() - 1;
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

    pub fn on_event(&mut self, pane: PaneId, event: &Event) {
        match event {
            Event::Output => {}
            Event::Title(title) if !title.trim().is_empty() => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.title = title.clone();
                }
            }
            Event::Title(_) => {}
            Event::Exited => self.close_pane(pane),
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

    pub fn render(&mut self, ui: &AppWindow) {
        let tabs: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|t| TabInfo {
                title: self
                    .panes
                    .get(&t.active)
                    .map_or("", |p| p.title.as_str())
                    .into(),
            })
            .collect();
        ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
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
        let pane = self.panes.get_mut(&id)?;
        let snap = pane.term.snapshot();

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
            .map(|(row, col, width)| SelRect {
                row: row as i32,
                col: col as i32,
                width: width as i32,
            })
            .collect();

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
            active,
            rows: pane.rows as i32,
            history: snap.history as i32,
            offset: snap.offset as i32,
        })
    }
}

/// Shell de cada painel: `NEMUX_SHELL` (ex.: `cmd.exe`) ou PowerShell.
pub fn shell_command() -> CommandBuilder {
    let shell = std::env::var("NEMUX_SHELL").unwrap_or_else(|_| "powershell.exe -NoLogo".into());
    let mut parts = shell.split_whitespace();
    let mut cmd = CommandBuilder::new(parts.next().unwrap_or("powershell.exe"));
    cmd.args(parts);
    if let Some(home) = std::env::var_os("USERPROFILE") {
        cmd.cwd(home);
    }
    cmd.env("TERM_PROGRAM", "nemux");
    cmd
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
