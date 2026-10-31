// Esconde o console extra no Windows nas builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod claude_settings;
mod config;
#[cfg(windows)]
mod conpty;
mod keys;
mod layout;
mod memory_view;
mod search;
mod selection;
mod session_view;
mod sessions;
mod terminal;
mod usage_view;
// Rodam cmd.exe de verdade no ConPTY: só no Windows.
#[cfg(all(test, windows))]
mod ui_tests;
mod window;
mod workspace;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{ModelRc, SharedString, VecModel};

use app::{App, Metrics};
use config::Config;
use keys::Mods;
use layout::{PaneId, Split};
use memory_view::MemoryView;
use nemux_memory::Memory;
use session_view::SessionsView;
use usage_view::UsageView;
use workspace::Workspace;

slint::include_modules!();

/// Abre um arquivo no programa padrão; trocável nos testes.
type Opener = Rc<RefCell<Box<dyn Fn(&Path)>>>;

type PendingOpen = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

/// Estado do app e os timers que repassam eventos; precisam viver enquanto a janela existir.
pub struct Controllers {
    app: Rc<RefCell<App>>,
    // Lido pelos testes de UI, que trocam a pasta e o comando das sessões.
    #[cfg_attr(not(all(test, windows)), allow(dead_code))]
    sessions: Rc<RefCell<SessionsView>>,
    /// Onde as abas são gravadas; `None` não grava nem restaura.
    workspace: Option<PathBuf>,
    /// "Restaurar abas ao abrir" (`restore` no config.toml); o menu muda na hora.
    restore: Rc<Cell<bool>>,
    /// Abre um arquivo no programa padrão do Windows (os testes trocam).
    #[cfg_attr(not(all(test, windows)), allow(dead_code))]
    opener: Opener,
    db: PathBuf,
    /// Abertura das abas à espera de a área dos painéis ter tamanho (ver `App::sized`).
    pending_open: PendingOpen,
    _timer: slint::Timer,
    _memory_timer: slint::Timer,
    _workspace_timer: slint::Timer,
    _usage_timer: slint::Timer,
    /// `settings.json` do Claude Code, onde "Uso do Claude na barra" liga a barra de
    /// status; `None` esconde a opção (os testes não mexem no do usuário).
    claude_settings: Rc<RefCell<Option<PathBuf>>>,
}

impl Controllers {
    /// Mostra "Uso do Claude na barra" no menu, ligado ao `settings.json` em `path`.
    pub fn use_claude_settings(&self, ui: &AppWindow, path: PathBuf) {
        ui.set_usage_bar(claude_settings::is_enabled(&path));
        ui.set_has_claude_settings(true);
        *self.claude_settings.borrow_mut() = Some(path);
    }

    pub fn open_tab(&self, ui: &AppWindow) {
        let mut app = self.app.borrow_mut();
        app.new_tab();
        app.render(ui);
    }

    /// Reabre as abas da última vez; sem elas, abre uma aba nova. Se a área dos
    /// painéis ainda não tem tamanho, espera por ele (`area-resized`).
    pub fn restore_or_open_tab(&self, ui: &AppWindow) {
        let open = {
            let (app, workspace, restore, weak) = (
                self.app.clone(),
                self.workspace.clone(),
                self.restore.clone(),
                ui.as_weak(),
            );
            move || {
                let saved = workspace
                    .as_deref()
                    .filter(|_| restore.get())
                    .and_then(Workspace::load);
                let mut app = app.borrow_mut();
                if saved.is_none_or(|saved| app.restore(&saved) == 0) {
                    app.new_tab();
                }
                if let Some(ui) = weak.upgrade() {
                    app.render(&ui);
                }
            }
        };
        if self.app.borrow().sized() {
            open();
        } else {
            *self.pending_open.borrow_mut() = Some(Box::new(open));
        }
    }

    /// Arquivos soltos na janela: os caminhos vão para o painel sob o ponteiro
    /// (`cursor`, na janela) ou, sem ele, para o painel ativo.
    pub fn drop_files(&self, ui: &AppWindow, paths: &[PathBuf], cursor: Option<(f32, f32)>) {
        drop_files(ui, &self.app, paths, cursor);
    }

    /// Grava as abas, com a sessão do Claude de cada painel.
    pub fn save_workspace(&self) {
        save_workspace(
            &self.app.borrow(),
            self.workspace.as_deref(),
            self.restore.get(),
            &self.db,
        );
    }
}

fn save_workspace(app: &App, path: Option<&Path>, restore: bool, db: &Path) {
    let Some(path) = path.filter(|_| restore) else {
        return;
    };
    let memory = Memory::open(db)
        .inspect_err(|e| eprintln!("nemux: não foi possível abrir {}: {e}", db.display()))
        .ok();
    let saved = Workspace::capture(app, |pane| {
        workspace::claude_session(memory.as_ref()?, pane)
    });
    if let Err(e) = saved.save(path) {
        eprintln!("nemux: não foi possível gravar {}: {e}", path.display());
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let controllers = init_controllers(&ui);
    controllers.restore_or_open_tab(&ui);
    ui.invoke_focus_terminal();
    let result = ui.run();
    // Com a janela fechada, mas antes de encerrar os terminais (e os Claudes neles).
    controllers.save_workspace();
    result
}

fn drop_files(ui: &AppWindow, app: &RefCell<App>, paths: &[PathBuf], cursor: Option<(f32, f32)>) {
    let mut app = app.borrow_mut();
    let pane = cursor.and_then(|(x, y)| app.pane_at(x - ui.get_area_x(), y - ui.get_area_y()));
    app.drop_files(pane, paths);
    app.render(ui);
    ui.invoke_focus_terminal();
}

/// Arrastar arquivos do Explorer para a janela. O Slint não repassa esses eventos, então
/// eles vêm direto do winit; sem winit (testes) isto não faz nada.
fn watch_file_drops(ui: &AppWindow, app: Rc<RefCell<App>>) {
    use slint::winit_030::winit::event::WindowEvent;
    use slint::winit_030::{EventResult, WinitWindowAccessor};

    let pending: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |_, event| {
        let Some(ui) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        match event {
            WindowEvent::HoveredFile(_) => ui.set_file_hover(true),
            WindowEvent::HoveredFileCancelled => ui.set_file_hover(false),
            WindowEvent::DroppedFile(path) => {
                ui.set_file_hover(false);
                // Um evento por arquivo, todos seguidos: junta-os e cola de uma vez.
                let first = pending.borrow().is_empty();
                pending.borrow_mut().push(path.clone());
                if first {
                    let (pending, weak, app) = (pending.clone(), weak.clone(), app.clone());
                    slint::Timer::single_shot(Duration::ZERO, move || {
                        let paths = std::mem::take(&mut *pending.borrow_mut());
                        if let Some(ui) = weak.upgrade() {
                            drop_files(&ui, &app, &paths, window::cursor_position(&ui));
                        }
                    });
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

/// Abre `path` no programa que o sistema associa ao tipo (no Windows, o Explorer
/// pergunta se não houver nenhum).
fn open_file(path: &Path) {
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(opener).arg(path).spawn() {
        eprintln!("nemux: não foi possível abrir {}: {e}", path.display());
    }
}

/// Medidas atuais da área dos painéis, lidas da interface.
fn metrics(ui: &AppWindow) -> Metrics {
    Metrics {
        width: ui.get_area_width(),
        height: ui.get_area_height(),
        cell_w: ui.get_cell_width(),
        cell_h: ui.get_cell_height(),
        pad: ui.get_pane_pad(),
        scrollbar: ui.get_scrollbar_width(),
    }
}

fn pane_id(id: i32) -> PaneId {
    id.max(0) as PaneId
}

/// Registra os callbacks da interface com o banco de memória, a configuração e as
/// abas salvas padrão.
pub fn init_controllers(ui: &AppWindow) -> Controllers {
    let controllers = init_controllers_with(
        ui,
        nemux_memory::default_path(),
        Some(&config::default_path()),
        Some(&workspace::default_path()),
    );
    if let Some(path) = claude_settings::default_path() {
        controllers.use_claude_settings(ui, path);
    }
    controllers
}

/// Registra os callbacks da interface. Não abre nenhuma aba, e o banco de memória só é
/// aberto quando o painel é usado (os testes de UI passam um banco temporário). Sem
/// `config`, usa a configuração padrão; um arquivo com erro também, com um aviso na janela.
/// Sem `workspace`, as abas não são gravadas nem restauradas.
pub fn init_controllers_with(
    ui: &AppWindow,
    db: PathBuf,
    config_path: Option<&Path>,
    workspace: Option<&Path>,
) -> Controllers {
    let config = match config_path.map(Config::load).transpose() {
        Ok(config) => config.unwrap_or_default(),
        Err(e) => {
            ui.set_config_error(format!("Configuração ignorada: {e}").into());
            Config::default()
        }
    };
    config.apply_theme(ui);
    let restore = Rc::new(Cell::new(config.restore.unwrap_or(true)));
    ui.set_restore_tabs(restore.get());
    ui.set_has_config(config_path.is_some());
    ui.set_themes(ModelRc::new(VecModel::from(config::theme_items())));
    ui.set_theme_index(config.theme_index() as i32);
    if config.overrides_theme() {
        ui.set_themes_note(
            "Algumas cores vêm de [colors] ou [app] no config.toml e ficam por cima do tema."
                .into(),
        );
    }
    ui.on_config_error_dismissed({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_config_error("".into());
                ui.invoke_focus_terminal();
            }
        }
    });

    let (tx, rx) = mpsc::channel();

    let panes_model = Rc::new(VecModel::default());
    ui.set_panes(ModelRc::from(panes_model.clone()));
    let tabs_model = Rc::new(VecModel::default());
    ui.set_tabs(ModelRc::from(tabs_model.clone()));
    let dividers_model = Rc::new(VecModel::default());
    ui.set_dividers(ModelRc::from(dividers_model.clone()));
    let mut app = App::new(tx, metrics(ui), panes_model, tabs_model, dividers_model);
    app.colors = config.terminal_colors();
    let shell = config.shell.clone();
    app.shell = Box::new(move || app::shell_command(shell.as_deref()));
    app.foreground = Box::new({
        let weak = ui.as_weak();
        move || weak.upgrade().is_none_or(|ui| window::is_foreground(&ui))
    });
    app.on_attention = Box::new({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                window::flash(&ui);
            }
        }
    });
    let app = Rc::new(RefCell::new(app));

    // Menu da engrenagem: tema, fonte e "restaurar abas" valem na hora e vão para o config.toml.
    let config = Rc::new(RefCell::new(config));
    let config_path = config_path.map(Path::to_path_buf);
    let save = {
        let (weak, config_path) = (ui.as_weak(), config_path.clone());
        move |what: &str, result: &dyn Fn(&Path) -> Result<(), String>| {
            if let (Some(ui), Some(path)) = (weak.upgrade(), &config_path)
                && let Err(e) = result(path)
            {
                ui.set_config_error(format!("Não foi possível gravar {what}: {e}").into());
            }
        }
    };
    let save = Rc::new(save);
    ui.on_theme_chosen({
        let (app, weak, config, save) = (app.clone(), ui.as_weak(), config.clone(), save.clone());
        move |i| {
            let (Some(ui), Some(preset)) = (weak.upgrade(), config::presets().get(i as usize))
            else {
                return;
            };
            let mut config = config.borrow_mut();
            config.theme = Some(preset.name.clone());
            config.apply_theme(&ui);
            ui.set_theme_index(i);
            {
                let mut app = app.borrow_mut();
                app.colors = config.terminal_colors();
                app.render(&ui);
            }
            save("o tema", &|path| config::save_theme(path, &preset.name));
        }
    });
    // O tamanho das células muda com a fonte; o `area-resized` redimensiona os terminais.
    ui.on_font_zoom({
        let (weak, config, save) = (ui.as_weak(), config.clone(), save.clone());
        move |step| {
            let Some(ui) = weak.upgrade() else { return };
            let mut config = config.borrow_mut();
            let current = config.font.size.unwrap_or(config::DEFAULT_FONT_SIZE);
            let size = match step {
                0 => None,
                _ => Some((current + step as f32).clamp(6.0, 72.0)),
            };
            if size == config.font.size {
                return;
            }
            config.font.size = size;
            config.apply_theme(&ui);
            save("a fonte", &|path| config::save_font_size(path, size));
        }
    });
    let claude_settings: Rc<RefCell<Option<PathBuf>>> = Rc::default();
    ui.on_usage_bar_toggled({
        let (weak, claude_settings) = (ui.as_weak(), claude_settings.clone());
        move |on| {
            let (Some(ui), Some(path)) = (weak.upgrade(), claude_settings.borrow().clone()) else {
                return;
            };
            let result = if on {
                claude_settings::nemux_command()
                    .and_then(|command| claude_settings::enable(&path, &command))
            } else {
                claude_settings::disable(&path)
            };
            match result {
                Ok(()) => ui.set_usage_bar(on),
                Err(e) => ui.set_config_error(e.into()),
            }
        }
    });
    ui.on_restore_toggled({
        let (weak, restore, save) = (ui.as_weak(), restore.clone(), save.clone());
        move |on| {
            restore.set(on);
            if let Some(ui) = weak.upgrade() {
                ui.set_restore_tabs(on);
            }
            save("\"restaurar abas\"", &|path| config::save_restore(path, on));
        }
    });
    let opener: Opener = Rc::new(RefCell::new(Box::new(open_file)));
    ui.on_open_config({
        let (weak, opener) = (ui.as_weak(), opener.clone());
        move || {
            let (Some(ui), Some(path)) = (weak.upgrade(), &config_path) else {
                return;
            };
            match config::ensure_exists(path) {
                Ok(()) => (opener.borrow())(path),
                Err(e) => {
                    ui.set_config_error(format!("Não foi possível criar {e}").into());
                }
            }
        }
    });

    watch_file_drops(ui, app.clone());

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
        move |i| on_change(&|app| app.select_tab(i.max(0) as usize))
    });
    ui.on_rename_tab({
        let on_change = on_change.clone();
        move |i, name| on_change(&|app| app.rename_tab(i.max(0) as usize, &name))
    });
    ui.on_move_tab({
        let on_change = on_change.clone();
        move |from, to| on_change(&|app| app.move_tab(from.max(0) as usize, to.max(0) as usize))
    });
    ui.on_close_tab({
        let on_change = on_change.clone();
        move |i| on_change(&|app| app.close_tab(i.max(0) as usize))
    });
    ui.on_split_right({
        let on_change = on_change.clone();
        move || {
            on_change(&|app| {
                app.split(Split::Right);
            })
        }
    });
    ui.on_split_down({
        let on_change = on_change.clone();
        move || {
            on_change(&|app| {
                app.split(Split::Down);
            })
        }
    });
    let pending_open: PendingOpen = Rc::default();
    ui.on_area_resized({
        let (on_change, weak) = (on_change.clone(), ui.as_weak());
        let (app, pending_open) = (app.clone(), pending_open.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let m = metrics(&ui);
            if app.borrow().tabs.is_empty() {
                app.borrow_mut().set_metrics(m);
                if app.borrow().sized()
                    && let Some(open) = pending_open.borrow_mut().take()
                {
                    open();
                }
            } else {
                on_change(&|app| app.set_metrics(m))
            }
        }
    });
    ui.on_scroll_lines({
        let on_change = on_change.clone();
        move |pane, lines| on_change(&|app| app.scroll(pane_id(pane), lines as isize))
    });
    ui.on_divider_drag({
        let on_change = on_change.clone();
        move |index, x, y| on_change(&|app| app.drag_divider(index.max(0) as usize, x, y))
    });
    ui.on_divider_reset({
        let on_change = on_change.clone();
        move |index| on_change(&|app| app.reset_divider(index.max(0) as usize))
    });
    ui.on_wheel({
        let on_change = on_change.clone();
        move |pane, lines, row, col| on_change(&|app| app.wheel(pane_id(pane), lines, row, col))
    });
    ui.on_scroll_to({
        let on_change = on_change.clone();
        move |pane, offset| on_change(&|app| app.scroll_to(pane_id(pane), offset.max(0) as usize))
    });
    ui.on_select_start({
        let on_change = on_change.clone();
        move |pane, row, col| on_change(&|app| app.start_selection(pane_id(pane), row, col))
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
        move |pane, row, col| on_change(&|app| app.select_word(pane_id(pane), row, col))
    });
    ui.on_secondary_click({
        let on_change = on_change.clone();
        move |pane| on_change(&|app| app.secondary_click(pane_id(pane)))
    });
    ui.on_search_changed({
        let on_change = on_change.clone();
        move |text| on_change(&|app| app.set_search_query(&text))
    });
    ui.on_search_step({
        let on_change = on_change.clone();
        move |up| on_change(&|app| app.search_step(up))
    });
    ui.on_search_closed({
        let on_change = on_change.clone();
        move || on_change(&|app| app.close_search())
    });
    ui.on_key_input({
        let on_change = on_change.clone();
        move |text: SharedString, ctrl, alt, shift| {
            let mods = Mods { ctrl, alt, shift };
            let handled = std::cell::Cell::new(false);
            on_change(&|app| handled.set(app.handle_key(&text, mods)));
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
                for (pane, event) in &events {
                    app.on_event(*pane, event);
                }
            });
        }
    });

    let memory = Rc::new(RefCell::new(MemoryView::new(db.clone())));
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

    let sessions = Rc::new(RefCell::new(SessionsView::new(sessions::projects_dir())));
    ui.on_sessions_refresh({
        let (sessions, weak) = (sessions.clone(), ui.as_weak());
        move || {
            if let Some(ui) = weak.upgrade() {
                sessions.borrow_mut().refresh(&ui);
            }
        }
    });
    ui.on_sessions_filter({
        let (sessions, weak) = (sessions.clone(), ui.as_weak());
        move || {
            if let Some(ui) = weak.upgrade() {
                sessions.borrow_mut().filter(&ui);
            }
        }
    });
    ui.on_session_open({
        let (sessions, weak, on_change) = (sessions.clone(), ui.as_weak(), on_change.clone());
        move |index| {
            let Some(ui) = weak.upgrade() else { return };
            let Some(cmd) = sessions.borrow().command(index.max(0) as usize) else {
                ui.set_sessions_status(
                    "Não dá para retomar esta sessão: a pasta dela não existe mais.".into(),
                );
                return;
            };
            on_change(&|app| app.new_tab_with(cmd.clone()));
            ui.set_sessions_open(false);
            ui.invoke_focus_terminal();
        }
    });

    // Grava as abas de tempos em tempos, para não perdê-las se o app não fechar direito.
    let workspace = workspace.map(Path::to_path_buf);
    let workspace_timer = slint::Timer::default();
    workspace_timer.start(slint::TimerMode::Repeated, Duration::from_secs(30), {
        let (app, workspace, restore, db) =
            (app.clone(), workspace.clone(), restore.clone(), db.clone());
        move || save_workspace(&app.borrow(), workspace.as_deref(), restore.get(), &db)
    });

    // Uso do Claude Code: limites na barra de abas e contexto em cada painel.
    let usage = RefCell::new(UsageView::new(db.clone()));
    let usage_timer = slint::Timer::default();
    let poll_usage = {
        let (app, weak) = (app.clone(), ui.as_weak());
        move || {
            let (Some(ui), Some(snap)) = (weak.upgrade(), usage.borrow_mut().poll()) else {
                return;
            };
            ui.set_usage_meters(ModelRc::new(VecModel::from(snap.meters)));
            ui.set_usage_detail(snap.detail.into());
            let mut app = app.borrow_mut();
            if app.contexts != snap.contexts {
                app.contexts = snap.contexts;
                if !app.tabs.is_empty() {
                    app.render(&ui);
                }
            }
        }
    };
    poll_usage();
    usage_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        poll_usage,
    );

    Controllers {
        app,
        sessions,
        workspace,
        restore,
        opener,
        db,
        pending_open,
        _timer: timer,
        _memory_timer: memory_timer,
        _workspace_timer: workspace_timer,
        _usage_timer: usage_timer,
        claude_settings,
    }
}
