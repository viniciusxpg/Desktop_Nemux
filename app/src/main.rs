// Esconde o console extra no Windows nas builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(windows)]
mod conpty;
mod keys;
mod layout;
mod memory_view;
mod selection;
mod session_view;
mod sessions;
mod terminal;
#[cfg(test)]
mod ui_tests;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{ModelRc, SharedString, VecModel};

use app::{App, Metrics};
use keys::Mods;
use layout::{PaneId, Split};
use memory_view::MemoryView;
use session_view::SessionsView;

slint::include_modules!();

/// Estado do app e os timers que repassam eventos; precisam viver enquanto a janela existir.
pub struct Controllers {
    app: Rc<RefCell<App>>,
    // Lido pelos testes de UI, que trocam a pasta e o comando das sessões.
    #[cfg_attr(not(test), allow(dead_code))]
    sessions: Rc<RefCell<SessionsView>>,
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

/// Registra os callbacks da interface com o banco de memória padrão.
pub fn init_controllers(ui: &AppWindow) -> Controllers {
    init_controllers_with_db(ui, nemux_memory::default_path())
}

/// Registra os callbacks da interface. Não abre nenhuma aba, e o banco de memória só é
/// aberto quando o painel é usado (os testes de UI passam um banco temporário).
pub fn init_controllers_with_db(ui: &AppWindow, db: PathBuf) -> Controllers {
    let (tx, rx) = mpsc::channel();

    let panes_model = Rc::new(VecModel::default());
    ui.set_panes(ModelRc::from(panes_model.clone()));
    let tabs_model = Rc::new(VecModel::default());
    ui.set_tabs(ModelRc::from(tabs_model.clone()));
    let dividers_model = Rc::new(VecModel::default());
    ui.set_dividers(ModelRc::from(dividers_model.clone()));
    let app = Rc::new(RefCell::new(App::new(
        tx,
        metrics(ui),
        panes_model,
        tabs_model,
        dividers_model,
    )));

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
    ui.on_area_resized({
        let (on_change, weak) = (on_change.clone(), ui.as_weak());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let m = metrics(&ui);
            on_change(&|app| app.set_metrics(m))
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

    Controllers {
        app,
        sessions,
        _timer: timer,
        _memory_timer: memory_timer,
    }
}
