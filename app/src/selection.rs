//! Seleção de texto na grade do terminal, em coordenadas da tela visível.

/// Seleção linear (como num editor de texto), com as duas pontas inclusivas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Onde o arraste começou.
    pub anchor: (u16, u16),
    /// Onde o ponteiro está agora.
    pub head: (u16, u16),
}

/// Caracteres que separam palavras no duplo clique. Barras, pontos e hífens
/// ficam de fora para que caminhos de arquivo e URLs sejam selecionados inteiros.
const WORD_DELIMITERS: &str = "()[]{}<>'\"`|,;│";

impl Selection {
    pub fn at(row: u16, col: u16) -> Self {
        Self {
            anchor: (row, col),
            head: (row, col),
        }
    }

    /// (início, fim) em ordem de leitura.
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Retângulos a destacar, um por linha: (linha, coluna, largura).
    pub fn rects(&self, cols: u16) -> Vec<(u16, u16, u16)> {
        let ((r0, c0), (r1, c1)) = self.ordered();
        (r0..=r1)
            .map(|row| {
                let start = if row == r0 { c0 } else { 0 };
                let end = if row == r1 {
                    c1
                } else {
                    cols.saturating_sub(1)
                };
                (row, start, end.saturating_sub(start) + 1)
            })
            .collect()
    }

    /// Texto selecionado. Linhas quebradas automaticamente pelo terminal são unidas,
    /// e os espaços no fim de cada linha são descartados.
    pub fn text(&self, screen: &vt100::Screen) -> String {
        let ((r0, c0), (r1, c1)) = self.ordered();
        screen.contents_between(r0, c0, r1, c1 + 1)
    }

    /// Seleção da palavra na célula (`row`, `col`), ou só a célula se ela estiver vazia.
    pub fn word(screen: &vt100::Screen, row: u16, col: u16) -> Self {
        let (_, cols) = screen.size();
        let is_word = |c: u16| {
            screen.cell(row, c).is_some_and(|cell| {
                let text = cell.contents();
                cell.is_wide_continuation()
                    || (!text.trim().is_empty()
                        && !text.chars().all(|ch| WORD_DELIMITERS.contains(ch)))
            })
        };
        if !is_word(col) {
            return Self::at(row, col);
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
            anchor: (row, start),
            head: (row, end),
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
            0,
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
        assert_eq!(s.rects(10), [(1, 4, 6), (2, 0, 10), (3, 0, 3)]);
        assert_eq!(Selection::at(0, 7).rects(10), [(0, 7, 1)]);
    }

    #[test]
    fn text_single_and_multi_line() {
        let p = screen(3, 20, "primeira linha\r\nsegunda\r\nterceira");
        let s = Selection {
            anchor: (0, 9),
            head: (0, 13),
        };
        assert_eq!(s.text(p.screen()), "linha");
        // Arrastando para trás, de "terc" até "linha".
        let s = Selection {
            anchor: (2, 3),
            head: (0, 9),
        };
        assert_eq!(s.text(p.screen()), "linha\nsegunda\nterc");
    }

    #[test]
    fn wrapped_lines_are_joined() {
        // 25 caracteres numa tela de 10 colunas: o terminal quebra sozinho.
        let p = screen(4, 10, "abcdefghijklmnopqrstuvwxy");
        let s = Selection {
            anchor: (0, 0),
            head: (2, 9),
        };
        assert_eq!(s.text(p.screen()), "abcdefghijklmnopqrstuvwxy");
    }

    #[test]
    fn double_click_selects_paths() {
        let p = screen(
            1,
            60,
            "erro em (C:\\Users\\Intel\\nemux\\src\\main.rs:42) agora",
        );
        let s = Selection::word(p.screen(), 0, 15);
        assert_eq!(
            s.text(p.screen()),
            "C:\\Users\\Intel\\nemux\\src\\main.rs:42"
        );
        let s = Selection::word(p.screen(), 0, 2);
        assert_eq!(s.text(p.screen()), "erro");
    }

    #[test]
    fn double_click_on_space_selects_one_cell() {
        let p = screen(1, 20, "a   b");
        assert_eq!(Selection::word(p.screen(), 0, 2), Selection::at(0, 2));
    }

    #[test]
    fn double_click_on_wide_chars() {
        let p = screen(1, 20, "x 日本語 y");
        let s = Selection::word(p.screen(), 0, 4);
        assert_eq!(s.text(p.screen()), "日本語");
    }
}
