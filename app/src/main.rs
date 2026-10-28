// Esconde o console extra no Windows nas builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod keys;
mod memory_view;
mod selection;
mod terminal;
#[cfg(test)]
mod ui_tests;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use portable_pty::CommandBuilder;
use slint::{Model, ModelRc, SharedString, VecModel};

use keys::Mods;
use memory_view::MemoryView;
use selection::Selection;
use terminal::{Event, Rgb, Run, Terminal};

slint::include_modules!();

struct Tab {
    id: u64,
    title: String,
    term: Terminal,
}

struct App {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    rows: u16,
    cols: u16,
    events: mpsc::Sender<(u64, Event)>,
    clipboard: Option<arboard::Clipboard>,
    /// Linhas mostradas na tela, para atualizar só as que mudaram.
    lines_model: Rc<VecModel<TermLine>>,
    shown: Vec<Vec<Run>>,
    /// Seleção e o id da aba onde ela foi feita.
    selection: Option<(u64, Selection)>,
    /// O botão esquerdo está pressionado sobre o terminal.
    selecting: bool,
    /// O ponteiro saiu da célula inicial (um clique simples não seleciona).
    dragged: bool,
    /// Texto capturado quando a seleção terminou; é o que será copiado.
    selected_text: Option<String>,
}

impl App {
    fn active_id(&self) -> Option<u64> {
        self.tabs.get(self.active).map(|t| t.id)
    }

    /// Converte coordenadas vindas da interface (podem sair da grade) numa célula válida.
    fn clamp_cell(&self, row: i32, col: i32) -> (u16, u16) {
        let row = row.clamp(0, self.rows.saturating_sub(1) as i32) as u16;
        let col = col.clamp(0, self.cols.saturating_sub(1) as i32) as u16;
        (row, col)
    }

    fn start_selection(&mut self, row: i32, col: i32) {
        let (row, col) = self.clamp_cell(row, col);
        self.selection = self.active_id().map(|id| (id, Selection::at(row, col)));
        self.selected_text = None;
        self.selecting = true;
        self.dragged = false;
    }

    fn move_selection(&mut self, row: i32, col: i32) {
        let cell = self.clamp_cell(row, col);
        if let (true, Some((_, sel))) = (self.selecting, self.selection.as_mut()) {
            sel.head = cell;
            self.dragged |= sel.head != sel.anchor;
        }
    }

    fn end_selection(&mut self) {
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

    fn select_word(&mut self, row: i32, col: i32) {
        let (row, col) = self.clamp_cell(row, col);
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let sel = tab
            .term
            .with_screen(|screen| Selection::word(screen, row, col));
        self.selection = Some((tab.id, sel));
        self.selecting = false;
        self.capture_selection();
    }

    fn capture_selection(&mut self) {
        let text = match (&self.selection, self.tabs.get(self.active)) {
            (Some((id, sel)), Some(tab)) if *id == tab.id => tab.term.with_screen(|s| sel.text(s)),
            _ => String::new(),
        };
        if text.trim().is_empty() {
            self.clear_selection();
        } else {
            self.selected_text = Some(text);
        }
    }

    fn clear_selection(&mut self) {
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

    /// Botão direito: copia se houver seleção, senão cola (como no Windows Terminal).
    fn secondary_click(&mut self) {
        if !self.copy() {
            self.paste();
        }
    }

    fn new_tab(&mut self) {
        self.new_tab_with(shell_command());
    }

    fn new_tab_with(&mut self, cmd: CommandBuilder) {
        let id = self.next_id;
        self.next_id += 1;
        let events = self.events.clone();
        match Terminal::spawn(cmd, self.rows, self.cols, move |e| {
            let _ = events.send((id, e));
        }) {
            Ok(term) => {
                self.tabs.push(Tab {
                    id,
                    title: format!("Terminal {}", id + 1),
                    term,
                });
                self.active = self.tabs.len() - 1;
            }
            Err(e) => eprintln!("nemux: não foi possível abrir o terminal: {e}"),
        }
    }

    /// Fecha a aba; o `Drop` do `Terminal` encerra o processo.
    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
    }

    fn active_term(&self) -> Option<&Terminal> {
        self.tabs.get(self.active).map(|t| &t.term)
    }

    fn select_offset(&mut self, offset: isize) {
        let n = self.tabs.len() as isize;
        if n > 0 {
            self.active = (self.active as isize + offset).rem_euclid(n) as usize;
        }
    }

    fn paste(&mut self) {
        let text = self.clipboard().and_then(|c| c.get_text().ok());
        if let (Some(text), Some(term)) = (text, self.active_term()) {
            term.scroll_to_bottom();
            term.paste(&text);
        }
    }

    fn render(&mut self, ui: &AppWindow) {
        let tabs: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|t| TabInfo {
                title: t.title.as_str().into(),
            })
            .collect();
        ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
        ui.set_active_tab(self.active as i32);

        let snap = match self.active_term() {
            Some(term) => term.snapshot(),
            None => terminal::Snapshot {
                lines: Vec::new(),
                cursor: None,
            },
        };
        let model = &self.lines_model;
        if model.row_count() != snap.lines.len() {
            model.set_vec(snap.lines.iter().map(|l| to_line(l)).collect::<Vec<_>>());
        } else {
            for (row, line) in snap.lines.iter().enumerate() {
                if self.shown.get(row) != Some(line) {
                    model.set_row_data(row, to_line(line));
                }
            }
        }
        self.shown = snap.lines;

        let (row, col) = snap.cursor.map_or((-1, 0), |(r, c)| (r as i32, c as i32));
        ui.set_cursor_row(row);
        ui.set_cursor_col(col);

        // A seleção só vale na aba onde foi feita.
        if self
            .selection
            .is_some_and(|(id, _)| Some(id) != self.active_id())
        {
            self.clear_selection();
        }
        let rects: Vec<SelRect> = self
            .selection
            .map(|(_, sel)| sel.rects(self.cols))
            .unwrap_or_default()
            .into_iter()
            .map(|(row, col, width)| SelRect {
                row: row as i32,
                col: col as i32,
                width: width as i32,
            })
            .collect();
        ui.set_selection(ModelRc::new(VecModel::from(rects)));
    }
}

/// Shell de cada aba: `NEMUX_SHELL` (ex.: `cmd.exe`) ou PowerShell.
fn shell_command() -> CommandBuilder {
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

fn to_line(runs: &[Run]) -> TermLine {
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

/// Atalhos do próprio Nemux. Devolve true se a tecla foi consumida.
fn handle_shortcut(app: &mut App, text: &str, mods: Mods) -> bool {
    let key = text.to_lowercase();
    match (mods.ctrl, mods.shift, mods.alt, key.as_str()) {
        (true, true, false, "c") => {
            app.copy();
        }
        // Ctrl+C só copia se houver seleção; sem ela, vai para o programa (interromper).
        (true, false, false, "c") if app.selected_text.is_some() => {
            app.copy();
        }
        (true, true, false, "t") => app.new_tab(),
        (true, true, false, "w") => app.close_tab(app.active),
        (true, _, false, "v") => app.paste(),
        (true, false, false, "\t") => app.select_offset(1),
        (true, true, false, "\t") | (true, true, false, "\u{19}") => app.select_offset(-1),
        // Shift+PageUp/PageDown: rola o histórico.
        (false, true, false, "\u{f72c}") | (false, true, false, "\u{f72d}") => {
            let page = app.rows.saturating_sub(1).max(1) as isize;
            if let Some(term) = app.active_term() {
                term.scroll(if key == "\u{f72c}" { page } else { -page });
            }
        }
        _ => return false,
    }
    true
}

/// Estado do app e o timer que repassa os eventos dos terminais; precisam viver enquanto a janela existir.
pub struct Controllers {
    app: Rc<RefCell<App>>,
    _timer: slint::Timer,
    _memory_timer: slint::Timer,
}

impl Controllers {
    pub fn open_tab(&self, ui: &AppWindow) {
        let mut app = self.app.borrow_mut();
        app.new_tab();
        app.render(ui);
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let controllers = init_controllers(&ui);
    controllers.open_tab(&ui);
    ui.invoke_focus_terminal();
    ui.run()
}

/// Registra os callbacks da interface com o banco de memória padrão.
pub fn init_controllers(ui: &AppWindow) -> Controllers {
    init_controllers_with_db(ui, nemux_memory::default_path())
}

/// Registra os callbacks da interface. Não abre nenhuma aba, e o banco de memória só é
/// aberto quando o painel é usado (os testes de UI passam um banco temporário).
pub fn init_controllers_with_db(ui: &AppWindow, db: PathBuf) -> Controllers {
    let (tx, rx) = mpsc::channel();

    let lines_model = Rc::new(VecModel::default());
    ui.set_lines(ModelRc::from(lines_model.clone()));

    let app = Rc::new(RefCell::new(App {
        tabs: Vec::new(),
        active: 0,
        next_id: 0,
        rows: ui.get_grid_rows().max(1) as u16,
        cols: ui.get_grid_cols().max(1) as u16,
        events: tx,
        clipboard: None,
        lines_model,
        shown: Vec::new(),
        selection: None,
        selecting: false,
        dragged: false,
        selected_text: None,
    }));

    let weak = ui.as_weak();
    let on_change = {
        let app = app.clone();
        move |f: &dyn Fn(&mut App)| {
            let mut app = app.borrow_mut();
            f(&mut app);
            if app.tabs.is_empty() {
                let _ = slint::quit_event_loop();
            } else if let Some(ui) = weak.upgrade() {
                app.render(&ui);
            }
        }
    };
    let on_change = Rc::new(on_change);

    ui.on_new_tab({
        let on_change = on_change.clone();
        move || on_change(&|app| app.new_tab())
    });
    ui.on_select_tab({
        let on_change = on_change.clone();
        move |i| on_change(&|app| app.active = (i as usize).min(app.tabs.len().saturating_sub(1)))
    });
    ui.on_close_tab({
        let on_change = on_change.clone();
        move |i| on_change(&|app| app.close_tab(i as usize))
    });
    ui.on_scroll_lines({
        let on_change = on_change.clone();
        move |lines| {
            on_change(&|app| {
                if let Some(term) = app.active_term() {
                    term.scroll(lines as isize);
                }
            })
        }
    });
    ui.on_grid_resized({
        let on_change = on_change.clone();
        move |cols, rows| {
            on_change(&|app| {
                app.cols = cols.max(1) as u16;
                app.rows = rows.max(1) as u16;
                for tab in &app.tabs {
                    tab.term.resize(app.rows, app.cols);
                }
            })
        }
    });
    ui.on_select_start({
        let on_change = on_change.clone();
        move |row, col| on_change(&|app| app.start_selection(row, col))
    });
    ui.on_select_move({
        let on_change = on_change.clone();
        move |row, col| on_change(&|app| app.move_selection(row, col))
    });
    ui.on_select_end({
        let on_change = on_change.clone();
        move || on_change(&|app| app.end_selection())
    });
    ui.on_select_word({
        let on_change = on_change.clone();
        move |row, col| on_change(&|app| app.select_word(row, col))
    });
    ui.on_secondary_click({
        let on_change = on_change.clone();
        move || on_change(&|app| app.secondary_click())
    });
    ui.on_key_input({
        let on_change = on_change.clone();
        move |text: SharedString, ctrl, alt, shift| {
            let mods = Mods { ctrl, alt, shift };
            let handled = std::cell::Cell::new(false);
            on_change(&|app| {
                if handle_shortcut(app, &text, mods) {
                    handled.set(true);
                    return;
                }
                let Some(term) = app.active_term() else {
                    return;
                };
                if let Some(bytes) = keys::encode(&text, mods, term.application_cursor()) {
                    term.scroll_to_bottom();
                    term.write(&bytes);
                    handled.set(true);
                    // Digitar descarta a seleção, como no Windows Terminal.
                    app.clear_selection();
                }
            });
            handled.get()
        }
    });

    // Junta os eventos das threads dos terminais e redesenha no máximo ~60 vezes por segundo.
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), {
        let on_change = on_change.clone();
        move || {
            let events: Vec<_> = rx.try_iter().collect();
            if events.is_empty() {
                return;
            }
            on_change(&|app| {
                for (id, event) in &events {
                    let Some(index) = app.tabs.iter().position(|t| t.id == *id) else {
                        continue;
                    };
                    match event {
                        Event::Output => {}
                        Event::Title(title) if !title.trim().is_empty() => {
                            app.tabs[index].title = title.clone();
                        }
                        Event::Title(_) => {}
                        Event::Exited => app.close_tab(index),
                    }
                }
            });
        }
    });

    let memory = Rc::new(RefCell::new(MemoryView::new(db)));
    let weak = ui.as_weak();
    ui.on_memory_refresh({
        let (memory, weak) = (memory.clone(), weak.clone());
        move || {
            if let Some(ui) = weak.upgrade() {
                memory.borrow_mut().refresh(&ui);
            }
        }
    });
    ui.on_memory_delete({
        let (memory, weak) = (memory.clone(), weak.clone());
        move |id| {
            if let Some(ui) = weak.upgrade() {
                memory.borrow_mut().delete(&ui, id);
            }
        }
    });
    // Com o painel aberto, mostra na hora o que o Claude salvar em qualquer aba.
    let memory_timer = slint::Timer::default();
    memory_timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), {
        let memory = memory.clone();
        move || {
            if let Some(ui) = weak.upgrade()
                && ui.get_memory_open()
            {
                memory.borrow_mut().poll(&ui);
            }
        }
    });

    Controllers {
        app,
        _timer: timer,
        _memory_timer: memory_timer,
    }
}
