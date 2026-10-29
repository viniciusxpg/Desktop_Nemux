//! Um terminal real: processo num PTY (ConPTY no Windows) + parser `vt100`.

use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

const SCROLLBACK: usize = 10_000;

/// Eventos que as threads do terminal enviam para a interface.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Chegou saída nova; a tela precisa ser redesenhada.
    Output,
    /// O programa definiu o título (OSC 0/2).
    Title(String),
    /// O shell informou a pasta atual (OSC 7 ou OSC 9;9).
    Cwd(PathBuf),
    /// O programa pediu atenção: sino (BEL) ou notificação (OSC 9, 99 ou 777).
    Attention,
    /// O processo terminou.
    Exited,
}

/// Respostas a consultas do programa e título pendente, coletados pelo parser.
#[derive(Default)]
pub struct Callbacks {
    replies: Vec<u8>,
    title: Option<String>,
    cwd: Option<PathBuf>,
    attention: bool,
}

impl vt100::Callbacks for Callbacks {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c, first) {
            // Posição do cursor (DSR 6). O ConPTY pergunta isso ao iniciar e espera a resposta.
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.replies
                    .extend(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
            }
            // Status do terminal (DSR 5): "tudo certo".
            (None, 'n', 5) => self.replies.extend(b"\x1b[0n"),
            // Atributos do dispositivo (DA1): VT100 com opções avançadas.
            (None, 'c', 0) => self.replies.extend(b"\x1b[?1;2c"),
            _ => {}
        }
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if let Some(cwd) = osc_cwd(params) {
            self.cwd = Some(cwd);
        } else if is_notification(params) {
            self.attention = true;
        }
    }

    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.attention = true;
    }
}

/// Notificações de terminal: `OSC 9 ; mensagem` (iTerm2; os `OSC 9 ; <número>` são
/// outros comandos, do ConEmu), `OSC 99` (kitty) e `OSC 777 ; notify` (Ghostty e outros).
fn is_notification(params: &[&[u8]]) -> bool {
    match params {
        [b"9", first, ..] => !first.is_empty() && !first.iter().all(u8::is_ascii_digit),
        [b"99", ..] => true,
        [b"777", b"notify", ..] => true,
        _ => false,
    }
}

/// Pasta atual vinda do shell: `OSC 7 ; file://host/C:/pasta` (com `%xx`) ou
/// `OSC 9 ; 9 ; "C:\pasta"` (o do Windows Terminal).
fn osc_cwd(params: &[&[u8]]) -> Option<PathBuf> {
    // O vt100 corta nos ';', que podem fazer parte do caminho: junta de volta.
    let rest = |from: usize| {
        params[from.min(params.len())..]
            .iter()
            .map(|p| String::from_utf8_lossy(p))
            .collect::<Vec<_>>()
            .join(";")
    };
    let path = match params {
        [b"7", ..] => {
            let url = rest(1);
            let after_host = url.strip_prefix("file://")?;
            let path = percent_decode(&after_host[after_host.find('/')?..]);
            // "/C:/pasta" no Windows.
            match path.as_bytes() {
                [b'/', _, b':', ..] => path[1..].to_owned(),
                _ => path,
            }
        }
        [b"9", b"9", ..] => rest(2).trim_matches('"').to_owned(),
        _ => return None,
    };
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub type Parser = vt100::Parser<Callbacks>;

pub struct Terminal {
    parser: Arc<Mutex<Parser>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Terminal {
    /// Inicia `cmd` num PTY de `rows` x `cols`. `on_event` é chamado de outras threads.
    pub fn spawn(
        cmd: CommandBuilder,
        rows: u16,
        cols: u16,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        // Antes do primeiro pseudoconsole: o portable-pty escolhe o ConPTY uma vez só.
        #[cfg(windows)]
        crate::conpty::ensure_loaded();
        let (rows, cols) = (non_zero(rows), non_zero(cols));
        let pair = native_pty_system().openpty(pty_size(rows, cols))?;
        let mut child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let parser = Arc::new(Mutex::new(Parser::new_with_callbacks(
            rows,
            cols,
            SCROLLBACK,
            Callbacks::default(),
        )));
        let on_event = Arc::new(on_event);

        {
            let (parser, writer, on_event) = (parser.clone(), writer.clone(), on_event.clone());
            thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let (replies, title, cwd, attention) = {
                        let mut parser = parser.lock().unwrap();
                        parser.process(&buf[..n]);
                        let cb = parser.callbacks_mut();
                        (
                            std::mem::take(&mut cb.replies),
                            cb.title.take(),
                            cb.cwd.take(),
                            std::mem::take(&mut cb.attention),
                        )
                    };
                    if !replies.is_empty() {
                        let mut w = writer.lock().unwrap();
                        let _ = w.write_all(&replies).and_then(|_| w.flush());
                    }
                    if let Some(title) = title {
                        on_event(Event::Title(title));
                    }
                    if let Some(cwd) = cwd {
                        on_event(Event::Cwd(cwd));
                    }
                    if attention {
                        on_event(Event::Attention);
                    }
                    on_event(Event::Output);
                }
            });
        }

        thread::spawn(move || {
            let _ = child.wait();
            on_event(Event::Exited);
        });

        Ok(Self {
            parser,
            writer,
            master: pair.master,
            killer,
        })
    }

    pub fn write(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes).and_then(|_| w.flush());
    }

    /// Cola texto, respeitando o modo "bracketed paste" quando o programa o ativou.
    pub fn paste(&self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.parser.lock().unwrap().screen().bracketed_paste() {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let (rows, cols) = (non_zero(rows), non_zero(cols));
        let mut parser = self.parser.lock().unwrap();
        if parser.screen().size() == (rows, cols) {
            return;
        }
        parser.set_size(rows, cols);
        let _ = self.master.resize(pty_size(rows, cols));
    }

    /// Rola o histórico: positivo sobe, negativo desce.
    pub fn scroll(&self, lines: isize) {
        let mut parser = self.parser.lock().unwrap();
        let screen = parser.screen_mut();
        let pos = screen.scrollback().saturating_add_signed(lines);
        screen.set_scrollback(pos);
    }

    /// Volta para o fim do histórico (tela ao vivo).
    pub fn scroll_to_bottom(&self) {
        self.scroll_to(0);
    }

    /// Mostra a tela `offset` linhas acima do fim do histórico (0 = ao vivo).
    pub fn scroll_to(&self, offset: usize) {
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_scrollback(offset);
    }

    /// Formato em que o programa quer receber o mouse, se ele ligou o mouse.
    pub fn mouse_reporting(&self) -> Option<vt100::MouseProtocolEncoding> {
        let parser = self.parser.lock().unwrap();
        let screen = parser.screen();
        (screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None)
            .then(|| screen.mouse_protocol_encoding())
    }

    /// O programa está na tela alternativa (a de `vim`, `less`, Claude Code), que não tem histórico.
    pub fn alternate_screen(&self) -> bool {
        self.parser.lock().unwrap().screen().alternate_screen()
    }

    pub fn application_cursor(&self) -> bool {
        self.parser.lock().unwrap().screen().application_cursor()
    }

    pub fn snapshot(&self, colors: &Colors) -> Snapshot {
        snapshot(self.parser.lock().unwrap().screen(), colors)
    }

    /// Acesso de leitura à tela (ex.: para extrair o texto selecionado).
    pub fn with_screen<T>(&self, f: impl FnOnce(&vt100::Screen) -> T) -> T {
        f(self.parser.lock().unwrap().screen())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

fn non_zero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

fn pty_size(rows: NonZeroU16, cols: NonZeroU16) -> PtySize {
    PtySize {
        rows: rows.get(),
        cols: cols.get(),
        pixel_width: 0,
        pixel_height: 0,
    }
}

pub type Rgb = (u8, u8, u8);

/// Cores padrão do texto e do fundo e as 16 cores ANSI (o resto das 256 é fixo).
#[derive(Debug, Clone, PartialEq)]
pub struct Colors {
    pub foreground: Rgb,
    pub background: Rgb,
    pub palette: [Rgb; 16],
}

impl Default for Colors {
    /// Esquema "Campbell" do Windows Terminal, com o fundo do tema do Nemux.
    fn default() -> Self {
        Self {
            foreground: (0xcc, 0xcc, 0xcc),
            background: (0x1e, 0x1e, 0x1e),
            palette: [
                (0x0c, 0x0c, 0x0c),
                (0xc5, 0x0f, 0x1f),
                (0x13, 0xa1, 0x0e),
                (0xc1, 0x9c, 0x00),
                (0x00, 0x37, 0xda),
                (0x88, 0x17, 0x98),
                (0x3a, 0x96, 0xdd),
                (0xcc, 0xcc, 0xcc),
                (0x76, 0x76, 0x76),
                (0xe7, 0x48, 0x56),
                (0x16, 0xc6, 0x0c),
                (0xf9, 0xf1, 0xa5),
                (0x3b, 0x78, 0xff),
                (0xb4, 0x00, 0x9e),
                (0x61, 0xd6, 0xd6),
                (0xf2, 0xf2, 0xf2),
            ],
        }
    }
}

impl Colors {
    /// Paleta de 256 cores do xterm: as 16 primeiras são as configuradas.
    pub fn palette(&self, i: u8) -> Rgb {
        match i {
            0..16 => self.palette[i as usize],
            16..232 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                (level(i / 36), level(i / 6 % 6), level(i % 6))
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                (v, v, v)
            }
        }
    }

    /// Converte a cor do vt100; `None` para a cor padrão. Negrito clareia as 8 cores básicas.
    fn color(&self, c: vt100::Color, bold: bool) -> Option<Rgb> {
        match c {
            vt100::Color::Default => None,
            vt100::Color::Rgb(r, g, b) => Some((r, g, b)),
            vt100::Color::Idx(i) if bold && i < 8 => Some(self.palette(i + 8)),
            vt100::Color::Idx(i) => Some(self.palette(i)),
        }
    }
}

/// Trecho de uma linha com o mesmo estilo, começando na coluna `col`.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub col: u16,
    pub width: u16,
    pub text: String,
    pub fg: Rgb,
    /// `None` = fundo padrão (não precisa desenhar).
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub lines: Vec<Vec<Run>>,
    /// (linha, coluna) do cursor, se visível.
    pub cursor: Option<(u16, u16)>,
    /// Linhas guardadas no histórico, acima da tela.
    pub history: usize,
    /// Quantas linhas acima do fim a tela está mostrando (0 = ao vivo).
    pub offset: usize,
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Rgb,
    bg: Option<Rgb>,
    bold: bool,
    italic: bool,
    underline: bool,
}

pub fn snapshot(screen: &vt100::Screen, colors: &Colors) -> Snapshot {
    let (rows, cols) = screen.size();
    let lines = (0..rows.get())
        .map(|row| line_runs(screen, row, cols.get(), colors))
        .collect();
    let cursor =
        (!screen.hide_cursor() && screen.scrollback() == 0).then(|| screen.cursor_position());
    Snapshot {
        lines,
        cursor,
        history: screen.scrollback_len(),
        offset: screen.scrollback(),
    }
}

fn line_runs(screen: &vt100::Screen, row: u16, cols: u16, colors: &Colors) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    // O trecho atual pode receber mais células? (Caracteres largos ficam isolados.)
    let mut open = false;
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let style = cell_style(cell, colors);
        let text = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        let width = if cell.is_wide() { 2 } else { 1 };

        match runs.last_mut() {
            Some(run) if open && width == 1 && style == run_style(run) => {
                run.text.push_str(text);
                run.width += 1;
            }
            _ => runs.push(Run {
                col,
                width,
                text: text.to_owned(),
                fg: style.fg,
                bg: style.bg,
                bold: style.bold,
                italic: style.italic,
                underline: style.underline,
            }),
        }
        open = width == 1;
    }
    // Espaços sem fundo no fim da linha não precisam ser desenhados.
    while let Some(last) = runs.last_mut() {
        if last.bg.is_some() || last.underline {
            break;
        }
        let trimmed = last.text.trim_end_matches(' ');
        if trimmed.is_empty() {
            runs.pop();
        } else {
            let removed = last.text.len() - trimmed.len();
            last.width -= removed as u16;
            last.text.truncate(trimmed.len());
            break;
        }
    }
    runs
}

fn run_style(run: &Run) -> Style {
    Style {
        fg: run.fg,
        bg: run.bg,
        bold: run.bold,
        italic: run.italic,
        underline: run.underline,
    }
}

fn cell_style(cell: &vt100::Cell, colors: &Colors) -> Style {
    let mut fg = colors
        .color(cell.fgcolor(), cell.bold())
        .unwrap_or(colors.foreground);
    let mut bg = colors.color(cell.bgcolor(), false);
    if cell.inverse() {
        let old_fg = fg;
        fg = bg.unwrap_or(colors.background);
        bg = Some(old_fg);
    }
    if cell.dim() {
        fg = mix(fg, bg.unwrap_or(colors.background));
    }
    Style {
        fg,
        bg,
        bold: cell.bold(),
        italic: cell.italic(),
        underline: cell.underline(),
    }
}

fn mix(a: Rgb, b: Rgb) -> Rgb {
    let m = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser(rows: u16, cols: u16) -> Parser {
        Parser::new_with_callbacks(non_zero(rows), non_zero(cols), 100, Callbacks::default())
    }

    #[test]
    fn groups_cells_into_styled_runs() {
        let mut p = parser(3, 20);
        p.process(b"ab\x1b[31mred\x1b[0m   \x1b[44m  \x1b[0m");
        let snap = snapshot(p.screen(), &Colors::default());
        let line = &snap.lines[0];
        let runs: Vec<_> = line.iter().map(|r| (r.col, r.text.as_str())).collect();
        assert_eq!(runs, [(0, "ab"), (2, "red"), (5, "   "), (8, "  ")]);
        assert_eq!(line[1].fg, Colors::default().palette(1));
        // O fundo azul no fim da linha é mantido; espaços sem fundo seriam cortados.
        assert_eq!(line[3].bg, Some(Colors::default().palette(4)));
        assert!(snap.lines[1].is_empty());
        assert_eq!(snap.cursor, Some((0, 10)));
    }

    #[test]
    fn trims_trailing_spaces() {
        let mut p = parser(1, 20);
        p.process(b"hi    ");
        let line = &snapshot(p.screen(), &Colors::default()).lines[0];
        assert_eq!(line.len(), 1);
        assert_eq!((line[0].text.as_str(), line[0].width), ("hi", 2));
    }

    #[test]
    fn wide_chars_get_their_own_run() {
        let mut p = parser(1, 20);
        p.process("a界b".as_bytes());
        let line = &snapshot(p.screen(), &Colors::default()).lines[0];
        let cols: Vec<_> = line
            .iter()
            .map(|r| (r.col, r.width, r.text.as_str()))
            .collect();
        assert_eq!(cols, [(0, 1, "a"), (1, 2, "界"), (3, 1, "b")]);
    }

    #[test]
    fn inverse_and_rgb() {
        let mut p = parser(1, 10);
        p.process(b"\x1b[7mX\x1b[0m\x1b[38;2;1;2;3mY");
        let line = &snapshot(p.screen(), &Colors::default()).lines[0];
        let c = Colors::default();
        assert_eq!((line[0].fg, line[0].bg), (c.background, Some(c.foreground)));
        assert_eq!(line[1].fg, (1, 2, 3));
    }

    #[test]
    fn answers_cursor_position_query() {
        let mut p = parser(5, 20);
        p.process(b"\r\nab\x1b[6n\x1b[c");
        assert_eq!(p.callbacks().replies, b"\x1b[2;3R\x1b[?1;2c");
    }

    #[test]
    fn captures_cwd() {
        let cwd = |seq: &[u8]| {
            let mut p = parser(1, 10);
            p.process(seq);
            p.callbacks_mut().cwd.take()
        };
        assert_eq!(
            cwd(b"\x1b]7;file://pc/C:/Users/Eu/meu%20proj;x\x07"),
            Some(PathBuf::from("C:/Users/Eu/meu proj;x"))
        );
        assert_eq!(
            cwd(b"\x1b]9;9;\"C:\\Users\\Eu\"\x1b\\"),
            Some(PathBuf::from("C:\\Users\\Eu"))
        );
        assert_eq!(
            cwd(b"\x1b]7;file:///home/eu\x07"),
            Some(PathBuf::from("/home/eu"))
        );
        assert_eq!(cwd(b"\x1b]9;4;1;50\x07"), None, "OSC 9;4 é progresso");
        assert_eq!(cwd(b"\x1b]1337;x\x07"), None);
    }

    #[test]
    fn captures_title() {
        let mut p = parser(1, 10);
        p.process(b"\x1b]0;meu titulo\x07");
        assert_eq!(p.callbacks().title.as_deref(), Some("meu titulo"));
    }

    #[test]
    fn hidden_cursor_and_scrollback() {
        let mut p = parser(2, 10);
        p.process(b"1\r\n2\r\n3\r\n4");
        let mut snap = snapshot(p.screen(), &Colors::default());
        assert_eq!(snap.lines[0][0].text, "3");
        assert_eq!((snap.history, snap.offset), (2, 0));
        p.screen_mut().set_scrollback(2);
        snap = snapshot(p.screen(), &Colors::default());
        assert_eq!(snap.lines[0][0].text, "1");
        assert_eq!((snap.history, snap.offset), (2, 2));
        assert_eq!(snap.cursor, None);
        // Além do topo, o vt100 limita ao tamanho do histórico.
        p.screen_mut().set_scrollback(99);
        assert_eq!(snapshot(p.screen(), &Colors::default()).offset, 2);
        p.screen_mut().set_scrollback(0);
        p.process(b"\x1b[?25l");
        assert_eq!(snapshot(p.screen(), &Colors::default()).cursor, None);
    }

    #[test]
    fn palette_cube_and_grays() {
        let c = Colors::default();
        assert_eq!(c.palette(16), (0, 0, 0));
        assert_eq!(c.palette(231), (255, 255, 255));
        assert_eq!(c.palette(232), (8, 8, 8));
        assert_eq!(c.palette(255), (238, 238, 238));
    }

    #[test]
    fn configured_colors() {
        let mut colors = Colors {
            foreground: (1, 1, 1),
            background: (2, 2, 2),
            ..Colors::default()
        };
        colors.palette[1] = (9, 0, 0);
        colors.palette[9] = (99, 0, 0);
        let mut p = parser(1, 20);
        p.process(b"a[31mb[1mc[0m[7md");
        let line = &snapshot(p.screen(), &colors).lines[0];
        let fg: Vec<_> = line.iter().map(|r| r.fg).collect();
        // Negrito usa a versão clara da paleta; inverso troca texto e fundo padrão.
        assert_eq!(fg, [(1, 1, 1), (9, 0, 0), (99, 0, 0), (2, 2, 2)]);
        assert_eq!(line[3].bg, Some((1, 1, 1)));
    }

    #[test]
    fn bell_and_notifications_ask_for_attention() {
        let attention = |seq: &[u8]| {
            let mut p = parser(1, 10);
            p.process(seq);
            std::mem::take(&mut p.callbacks_mut().attention)
        };
        assert!(attention(b"\x07"));
        assert!(attention(
            b"\x1b]9;\n\nClaude is waiting for your input\x07"
        ));
        assert!(attention(b"\x1b]9;pronto\x1b\\"));
        assert!(attention(b"\x1b]99;i=1:d=0;Claude Code\x1b\\"));
        assert!(attention(b"\x1b]777;notify;Claude Code;pronto\x07"));
        assert!(!attention(b"texto"));
        assert!(!attention(b"\x1b]9;4;1;50\x07"), "OSC 9;4 é progresso");
        assert!(!attention(b"\x1b]9;9;\"C:\\x\"\x07"), "OSC 9;9 é a pasta");
        assert!(!attention(b"\x1b]0;titulo\x07"));
    }

    /// O ConPTY repassa o sino e o OSC 9 de um programa de verdade.
    #[test]
    fn real_process_bell_and_osc9() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let espera = |script: &str| {
            let (tx, rx) = mpsc::channel();
            let mut cmd = CommandBuilder::new("powershell.exe");
            cmd.args(["-NoLogo", "-NoProfile", "-Command", script]);
            let tx = Mutex::new(tx);
            let _term = Terminal::spawn(cmd, 10, 40, move |e| {
                let _ = tx.lock().unwrap().send(e);
            })
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(Event::Attention) => return true,
                    Ok(Event::Exited) | Err(_) => return false,
                    Ok(_) => {}
                }
            }
        };
        assert!(espera("[Console]::Write([char]7); Start-Sleep 1"), "BEL");
        assert!(
            espera("[Console]::Write(\"$([char]27)]9;`n`nola$([char]7)\"); Start-Sleep 1"),
            "OSC 9"
        );
    }

    /// Roda um processo de verdade no ConPTY e confere a saída.
    #[test]
    fn runs_real_process() {
        use std::sync::mpsc;
        use std::time::Duration;

        let (tx, rx) = mpsc::channel();
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/c", "echo nemux-ok"]);
        let tx = Mutex::new(tx);
        let term = Terminal::spawn(cmd, 10, 40, move |e| {
            let _ = tx.lock().unwrap().send(e);
        })
        .unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(remaining) {
                Ok(Event::Exited) => break,
                Ok(_) => {}
                Err(_) => panic!("o processo não terminou a tempo"),
            }
        }
        // A saída pode chegar logo depois do fim do processo.
        let screen_text = || -> String {
            term.snapshot(&Colors::default())
                .lines
                .iter()
                .flatten()
                .map(|r| r.text.as_str())
                .collect()
        };
        for _ in 0..50 {
            if screen_text().contains("nemux-ok") {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("saída não encontrada: {:?}", screen_text());
    }
}
