//! Seleção de texto na grade do terminal.
//!
//! As linhas são contadas a partir do topo do histórico, como na busca
//! (`search::top_line`): a seleção continua no mesmo texto quando a tela rola
//! (inclusive pelo arraste além da borda, que rola o histórico).

use crate::search::top_line;

/// Seleção linear (como num editor de texto), com as duas pontas inclusivas:
/// (linha a partir do topo do histórico, coluna).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Onde o arraste começou.
    pub anchor: (usize, u16),
    /// Onde o ponteiro está agora.
    pub head: (usize, u16),
}

/// Caracteres que separam palavras no duplo clique. Barras, pontos e hífens
/// ficam de fora para que caminhos de arquivo e URLs sejam selecionados inteiros.
const WORD_DELIMITERS: &str = "()[]{}<>'\"`|,;│";

impl Selection {
    pub fn at(line: usize, col: u16) -> Self {
        Self {
            anchor: (line, col),
            head: (line, col),
        }
    }

    /// (início, fim) em ordem de leitura.
    pub fn ordered(&self) -> ((usize, u16), (usize, u16)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Retângulos à vista, um por linha: (linha da tela, coluna, largura). A tela
    /// mostra `rows` linhas a partir da linha `top` do histórico.
    pub fn rects(&self, cols: u16, top: usize, rows: u16) -> Vec<(u16, u16, u16)> {
        let ((l0, c0), (l1, c1)) = self.ordered();
        let last_visible = (top + rows as usize).saturating_sub(1);
        (l0.max(top)..=l1.min(last_visible))
            .map(|line| {
                let start = if line == l0 { c0 } else { 0 };
                let end = if line == l1 {
                    c1
                } else {
                    cols.saturating_sub(1)
                };
                ((line - top) as u16, start, end.saturating_sub(start) + 1)
            })
            .collect()
    }

    /// Texto selecionado. Linhas quebradas automaticamente pelo terminal são unidas,
    /// e os espaços no fim de cada linha são descartados. O `vt100` só lê as linhas à
    /// vista: o histórico é lido uma tela por vez, mudando a rolagem e a devolvendo.
    pub fn text(&self, screen: &mut vt100::Screen) -> String {
        let ((l0, c0), (l1, c1)) = self.ordered();
        let saved = screen.scrollback();
        let history = screen.scrollback_len();
        let (rows, cols) = (screen.size().0.get() as usize, screen.size().1.get());
        let mut text = String::new();
        let mut line = l0;
        while line <= l1 {
            // Põe `line` no topo da tela (ou o mais perto disso, no fim do histórico).
            screen.set_scrollback(history.saturating_sub(line));
            let top = top_line(screen);
            if line < top || line >= top + rows {
                break; // linha que não existe mais
            }
            let last = l1.min(top + rows - 1);
            let (r0, r1) = ((line - top) as u16, (last - top) as u16);
            let start = if line == l0 { c0 } else { 0 };
            let end = if last == l1 { c1 + 1 } else { cols };
            text.push_str(&screen.contents_between(r0, start, r1, end));
            if last < l1 && !screen.row_wrapped(r1) {
                text.push('\n');
            }
            line = last + 1;
        }
        screen.set_scrollback(saved);
        text
    }

    /// Seleção da palavra na célula (`row`, `col`) da tela, ou só a célula se ela
    /// estiver vazia.
    pub fn word(screen: &vt100::Screen, row: u16, col: u16) -> Self {
        let (_, cols) = screen.size();
        let line = top_line(screen) + row as usize;
        let is_word = |c: u16| {
            screen.cell(row, c).is_some_and(|cell| {
                let text = cell.contents();
                cell.is_wide_continuation()
                    || (!text.trim().is_empty()
                        && !text.chars().all(|ch| WORD_DELIMITERS.contains(ch)))
            })
        };
        if !is_word(col) {
            return Self::at(line, col);
        }
        let mut start = col;
        while start > 0 && is_word(start - 1) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < cols.get() && is_word(end + 1) {
            end += 1;
        }
        Self {
            anchor: (line, start),
            head: (line, end),
        }
    }

    /// Seleção da linha da tela `row` inteira (triplo clique), com as linhas que o
    /// terminal quebrou junto dela (as que estão à vista).
    pub fn line(screen: &vt100::Screen, row: u16) -> Self {
        let (rows, cols) = screen.size();
        let top = top_line(screen);
        let mut first = row;
        while first > 0 && screen.row_wrapped(first - 1) {
            first -= 1;
        }
        let mut last = row;
        while last + 1 < rows.get() && screen.row_wrapped(last) {
            last += 1;
        }
        Self {
            anchor: (top + first as usize, 0),
            head: (top + last as usize, cols.get() - 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;

    fn screen(rows: u16, cols: u16, text: &str) -> vt100::Parser {
        let mut p = vt100::Parser::new(
            NonZeroU16::new(rows).unwrap(),
            NonZeroU16::new(cols).unwrap(),
            100,
        );
        p.process(text.as_bytes());
        p
    }

    #[test]
    fn order_does_not_depend_on_drag_direction() {
        let s = Selection {
            anchor: (2, 5),
            head: (0, 3),
        };
        assert_eq!(s.ordered(), ((0, 3), (2, 5)));
    }

    #[test]
    fn rects_cover_lines_between() {
        let s = Selection {
            anchor: (1, 4),
            head: (3, 2),
        };
        assert_eq!(s.rects(10, 0, 5), [(1, 4, 6), (2, 0, 10), (3, 0, 3)]);
        assert_eq!(Selection::at(0, 7).rects(10, 0, 5), [(0, 7, 1)]);
        // Com a tela começando na linha 2 do histórico e 1 linha de altura: só a linha 2.
        assert_eq!(s.rects(10, 2, 1), [(0, 0, 10)]);
        assert!(s.rects(10, 4, 3).is_empty());
    }

    #[test]
    fn text_single_and_multi_line() {
        let mut p = screen(3, 20, "primeira linha\r\nsegunda\r\nterceira");
        let s = Selection {
            anchor: (0, 9),
            head: (0, 13),
        };
        assert_eq!(s.text(p.screen_mut()), "linha");
        // Arrastando para trás, de "terc" até "linha".
        let s = Selection {
            anchor: (2, 3),
            head: (0, 9),
        };
        assert_eq!(s.text(p.screen_mut()), "linha\nsegunda\nterc");
    }

    #[test]
    fn wrapped_lines_are_joined() {
        // 25 caracteres numa tela de 10 colunas: o terminal quebra sozinho.
        let mut p = screen(4, 10, "abcdefghijklmnopqrstuvwxy");
        let s = Selection {
            anchor: (0, 0),
            head: (2, 9),
        };
        assert_eq!(s.text(p.screen_mut()), "abcdefghijklmnopqrstuvwxy");
    }

    #[test]
    fn double_click_selects_paths() {
        let mut p = screen(
            1,
            60,
            "erro em (C:\\Users\\Intel\\nemux\\src\\main.rs:42) agora",
        );
        let s = Selection::word(p.screen(), 0, 15);
        assert_eq!(
            s.text(p.screen_mut()),
            "C:\\Users\\Intel\\nemux\\src\\main.rs:42"
        );
        let s = Selection::word(p.screen(), 0, 2);
        assert_eq!(s.text(p.screen_mut()), "erro");
    }

    #[test]
    fn text_from_history() {
        // 10 linhas numa tela de 3: as 7 primeiras vão para o histórico.
        let text: Vec<String> = (0..10).map(|i| format!("linha {i}")).collect();
        let mut p = screen(3, 20, &text.join("\r\n"));
        let s = Selection {
            anchor: (1, 6),
            head: (8, 6),
        };
        let esperado = (1..=8).map(|i| format!("linha {i}")).collect::<Vec<_>>();
        let esperado = esperado.join("\n")[6..].to_string();
        assert_eq!(s.text(p.screen_mut()), esperado);
        // A rolagem volta a ser a de antes.
        assert_eq!(p.screen().scrollback(), 0);
    }

    #[test]
    fn wrapped_line_split_between_screens() {
        // "abcdefghij" quebrada em 5+5 colunas, com a quebra entre duas leituras.
        let mut p = screen(2, 5, "x\r\ny\r\nabcdefghij\r\nfim");
        let top = top_line(p.screen());
        let wrapped = top - 1; // linha do histórico com "abcde"
        let s = Selection {
            anchor: (wrapped, 0),
            head: (wrapped + 1, 4),
        };
        assert_eq!(s.text(p.screen_mut()), "abcdefghij");
    }

    #[test]
    fn triple_click_selects_wrapped_line() {
        let mut p = screen(5, 10, "antes\r\nabcdefghijklmnopqrstuvwxy\r\ndepois");
        let s = Selection::line(p.screen(), 2);
        assert_eq!(s.ordered(), ((1, 0), (3, 9)));
        assert_eq!(s.text(p.screen_mut()), "abcdefghijklmnopqrstuvwxy");
        let s = Selection::line(p.screen(), 0);
        assert_eq!(s.text(p.screen_mut()), "antes");
    }

    #[test]
    fn double_click_on_space_selects_one_cell() {
        let p = screen(1, 20, "a   b");
        assert_eq!(Selection::word(p.screen(), 0, 2), Selection::at(0, 2));
    }

    #[test]
    fn double_click_on_wide_chars() {
        let mut p = screen(1, 20, "x 日本語 y");
        let s = Selection::word(p.screen(), 0, 4);
        assert_eq!(s.text(p.screen_mut()), "日本語");
    }
}
