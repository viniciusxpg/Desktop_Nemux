//! Busca de texto no histórico e na tela de um terminal.
//!
//! As linhas são contadas a partir do topo do histórico (0 = a mais antiga guardada):
//! essa conta não muda quando chega saída nova, só quando o histórico enche e
//! descarta as linhas mais antigas.

/// Uma ocorrência: linha (a partir do topo do histórico), coluna e largura em células.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub line: usize,
    pub col: u16,
    pub width: u16,
}

/// Texto procurado, já em minúsculas (a busca ignora maiúsculas e minúsculas).
fn needle(query: &str) -> Vec<char> {
    query.chars().flat_map(char::to_lowercase).collect()
}

/// Ocorrências numa linha da tela visível: (coluna, largura).
fn row_matches(screen: &vt100::Screen, row: u16, needle: &[char]) -> Vec<(u16, u16)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let (_, cols) = screen.size();
    // Cada caractere da linha com a coluna e a largura da célula de onde veio.
    let mut chars: Vec<(char, u16, u16)> = Vec::new();
    for col in 0..cols.get() {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let width = if cell.is_wide() { 2 } else { 1 };
        let text = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        for c in text.chars().flat_map(char::to_lowercase) {
            chars.push((c, col, width));
        }
    }
    let mut found = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|((c, ..), n)| c == n)
        {
            let (_, start, _) = chars[i];
            let (_, last, last_width) = chars[i + needle.len() - 1];
            found.push((start, last + last_width - start));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    found
}

/// Todas as ocorrências de `query` no histórico e na tela, de cima para baixo.
/// Percorre o histórico mudando a posição de rolagem e a devolve como estava.
pub fn find_all(screen: &mut vt100::Screen, query: &str) -> Vec<Match> {
    let needle = needle(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let saved = screen.scrollback();
    let history = screen.scrollback_len();
    let rows = screen.size().0.get() as usize;
    let mut found = Vec::new();
    // Próxima linha ainda não lida; a tela mostra `rows` linhas a partir de `history - offset`.
    let mut next = 0;
    while next < history + rows {
        let offset = history.saturating_sub(next);
        screen.set_scrollback(offset);
        let top = history - offset;
        for row in 0..rows {
            let line = top + row;
            if line < next {
                continue;
            }
            for (col, width) in row_matches(screen, row as u16, &needle) {
                found.push(Match { line, col, width });
            }
        }
        next = top + rows;
    }
    screen.set_scrollback(saved);
    found
}

/// Ocorrências na tela como ela está agora: (linha da tela, coluna, largura).
pub fn visible(screen: &vt100::Screen, query: &str) -> Vec<(u16, u16, u16)> {
    let needle = needle(query);
    (0..screen.size().0.get())
        .flat_map(|row| {
            row_matches(screen, row, &needle)
                .into_iter()
                .map(move |(col, width)| (row, col, width))
        })
        .collect()
}

/// Primeira linha da tela, contada a partir do topo do histórico.
pub fn top_line(screen: &vt100::Screen) -> usize {
    screen.scrollback_len() - screen.scrollback()
}

/// Posição de rolagem que mostra a linha `line`: a atual, se ela já está à vista,
/// senão uma que a deixa perto do meio da tela.
pub fn offset_showing(screen: &vt100::Screen, line: usize) -> usize {
    let history = screen.scrollback_len();
    let rows = screen.size().0.get() as usize;
    let top = top_line(screen);
    if (top..top + rows).contains(&line) {
        return screen.scrollback();
    }
    let top = line.saturating_sub(rows / 2);
    history.saturating_sub(top)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;

    fn parser(rows: u16, cols: u16, scrollback: usize, text: &str) -> vt100::Parser {
        let mut p = vt100::Parser::new(
            NonZeroU16::new(rows).unwrap(),
            NonZeroU16::new(cols).unwrap(),
            scrollback,
        );
        p.process(text.as_bytes());
        p
    }

    /// 30 linhas numeradas numa tela de 5: 25 vão para o histórico.
    fn numbered() -> vt100::Parser {
        let text: Vec<String> = (0..30).map(|i| format!("linha {i}")).collect();
        parser(5, 20, 100, &text.join("\r\n"))
    }

    #[test]
    fn finds_in_history_and_screen_and_restores_scroll() {
        let mut p = numbered();
        p.screen_mut().set_scrollback(3);
        let found = find_all(p.screen_mut(), "linha 2");
        let lines: Vec<usize> = found.iter().map(|m| m.line).collect();
        // "linha 2" e "linha 20".."linha 29".
        assert_eq!(lines, [2, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29]);
        assert_eq!(
            found[0],
            Match {
                line: 2,
                col: 0,
                width: 7
            }
        );
        assert_eq!(p.screen().scrollback(), 3);
    }

    #[test]
    fn ignores_case_and_finds_several_per_line() {
        let mut p = parser(3, 30, 0, "Erro: erro de novo ERRO");
        let found = find_all(p.screen_mut(), "erro");
        let cols: Vec<u16> = found.iter().map(|m| m.col).collect();
        assert_eq!(cols, [0, 6, 19]);
        assert!(find_all(p.screen_mut(), "").is_empty());
        assert!(find_all(p.screen_mut(), "nada").is_empty());
    }

    #[test]
    fn wide_chars_and_accents() {
        let mut p = parser(2, 30, 0, "x 日本語 y ação");
        assert_eq!(
            find_all(p.screen_mut(), "本語"),
            [Match {
                line: 0,
                col: 4,
                width: 4
            }]
        );
        assert_eq!(
            find_all(p.screen_mut(), "AÇÃO"),
            [Match {
                line: 0,
                col: 11,
                width: 4
            }]
        );
    }

    #[test]
    fn visible_matches_follow_the_scroll() {
        let mut p = numbered();
        assert_eq!(
            visible(p.screen(), "linha 2"),
            [(0, 0, 7), (1, 0, 7), (2, 0, 7), (3, 0, 7), (4, 0, 7)]
        );
        p.screen_mut().set_scrollback(25);
        assert_eq!(top_line(p.screen()), 0);
        assert_eq!(visible(p.screen(), "linha 2"), [(2, 0, 7)]);
    }

    #[test]
    fn offset_centers_lines_out_of_view() {
        let mut p = numbered();
        // Ao vivo, a tela mostra as linhas 25..30.
        assert_eq!(offset_showing(p.screen(), 27), 0);
        // A linha 2 fica no meio: topo na linha 0.
        assert_eq!(offset_showing(p.screen(), 2), 25);
        assert_eq!(offset_showing(p.screen(), 12), 15);
        p.screen_mut().set_scrollback(15);
        assert_eq!(offset_showing(p.screen(), 11), 15);
    }
}
