//! Painéis divididos: a árvore de painéis de uma aba e a geometria de cada um.
//!
//! O Slint não tem componentes recursivos, então a árvore fica aqui e a
//! interface recebe só a lista de retângulos já calculados.

pub type PaneId = u64;

/// Onde o novo painel aparece ao dividir.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Split {
    /// Ao lado, à direita (divisória vertical).
    Right,
    /// Abaixo (divisória horizontal).
    Down,
}

/// Direção para mover o foco entre painéis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// As duas metades ao dividir, com `gap` px de divisória entre elas.
    pub fn halves(&self, split: Split, gap: f32) -> (Rect, Rect) {
        match split {
            Split::Right => {
                let w = ((self.w - gap) / 2.0).floor().max(0.0);
                let b_x = self.x + w + gap;
                (
                    Rect::new(self.x, self.y, w, self.h),
                    Rect::new(b_x, self.y, (self.right() - b_x).max(0.0), self.h),
                )
            }
            Split::Down => {
                let h = ((self.h - gap) / 2.0).floor().max(0.0);
                let b_y = self.y + h + gap;
                (
                    Rect::new(self.x, self.y, self.w, h),
                    Rect::new(self.x, b_y, self.w, (self.bottom() - b_y).max(0.0)),
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split {
        split: Split,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    /// Painéis na ordem de leitura (esquerda/cima primeiro).
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect_leaves(out);
                b.collect_leaves(out);
            }
        }
    }

    /// Divide o painel `target`; `new` fica à direita ou abaixo dele.
    pub fn split(&mut self, target: PaneId, split: Split, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split {
                    split,
                    a: Box::new(Node::Leaf(target)),
                    b: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, split, new) || b.split(target, split, new),
        }
    }

    /// Remove o painel; o irmão dele ocupa o espaço. Não remove o último painel.
    pub fn remove(&mut self, target: PaneId) -> bool {
        let Node::Split { a, b, .. } = self else {
            return false;
        };
        let is_target = |n: &Node| matches!(n, Node::Leaf(id) if *id == target);
        let sibling = if is_target(a) {
            Some(std::mem::replace(b.as_mut(), Node::Leaf(target)))
        } else if is_target(b) {
            Some(std::mem::replace(a.as_mut(), Node::Leaf(target)))
        } else {
            None
        };
        match sibling {
            Some(node) => {
                *self = node;
                true
            }
            None => a.remove(target) || b.remove(target),
        }
    }

    /// Retângulo de cada painel dentro de `area`, com `gap` px entre painéis vizinhos.
    pub fn rects(&self, area: Rect, gap: f32) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.collect_rects(area, gap, &mut out);
        out
    }

    fn collect_rects(&self, area: Rect, gap: f32, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, area)),
            Node::Split { split, a, b } => {
                let (ra, rb) = area.halves(*split, gap);
                a.collect_rects(ra, gap, out);
                b.collect_rects(rb, gap, out);
            }
        }
    }
}

/// Painel vizinho de `from` na direção pedida: o mais próximo entre os que ficam
/// daquele lado e se sobrepõem a ele no outro eixo.
pub fn neighbor(rects: &[(PaneId, Rect)], from: PaneId, dir: Direction) -> Option<PaneId> {
    const EPS: f32 = 0.5;
    let (_, r) = *rects.iter().find(|(id, _)| *id == from)?;
    let overlaps_v = |o: &Rect| o.y < r.bottom() - EPS && o.bottom() > r.y + EPS;
    let overlaps_h = |o: &Rect| o.x < r.right() - EPS && o.right() > r.x + EPS;
    let center = |o: &Rect| (o.x + o.w / 2.0, o.y + o.h / 2.0);
    let (cx, cy) = center(&r);

    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, o)| {
            let distance = match dir {
                Direction::Left if o.right() <= r.x + EPS && overlaps_v(o) => r.x - o.right(),
                Direction::Right if o.x >= r.right() - EPS && overlaps_v(o) => o.x - r.right(),
                Direction::Up if o.bottom() <= r.y + EPS && overlaps_h(o) => r.y - o.bottom(),
                Direction::Down if o.y >= r.bottom() - EPS && overlaps_h(o) => o.y - r.bottom(),
                _ => return None,
            };
            // Desempate: o painel mais alinhado com o centro do atual.
            let (ox, oy) = center(o);
            let offset = match dir {
                Direction::Left | Direction::Right => (oy - cy).abs(),
                Direction::Up | Direction::Down => (ox - cx).abs(),
            };
            Some((distance, offset, *id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
        .map(|(_, _, id)| id)
}

/// Colunas e linhas de terminal que cabem num painel, com `pad` px de margem em
/// volta e `scrollbar` px reservados à direita para a barra de rolagem.
pub fn grid_size(rect: Rect, cell_w: f32, cell_h: f32, pad: f32, scrollbar: f32) -> (u16, u16) {
    let fit =
        |space: f32, cell: f32| (space / cell.max(1.0)).floor().clamp(1.0, u16::MAX as f32) as u16;
    (
        fit(rect.w - 2.0 * pad - scrollbar, cell_w),
        fit(rect.h - 2.0 * pad, cell_h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 600.0,
    };

    #[test]
    fn single_pane_fills_area() {
        assert_eq!(Node::Leaf(1).rects(AREA, 2.0), [(1, AREA)]);
    }

    #[test]
    fn split_right_and_down() {
        let mut n = Node::Leaf(1);
        assert!(n.split(1, Split::Right, 2));
        assert!(n.split(2, Split::Down, 3));
        assert!(!n.split(9, Split::Down, 4));
        assert_eq!(n.leaves(), [1, 2, 3]);

        let rects = n.rects(AREA, 2.0);
        assert_eq!(rects[0], (1, Rect::new(0.0, 0.0, 499.0, 600.0)));
        assert_eq!(rects[1], (2, Rect::new(501.0, 0.0, 499.0, 299.0)));
        assert_eq!(rects[2], (3, Rect::new(501.0, 301.0, 499.0, 299.0)));
    }

    #[test]
    fn remove_gives_space_to_sibling() {
        let mut n = Node::Leaf(1);
        n.split(1, Split::Right, 2);
        n.split(2, Split::Down, 3);

        assert!(n.remove(2));
        assert_eq!(n.leaves(), [1, 3]);
        assert_eq!(
            n.rects(AREA, 2.0)[1],
            (3, Rect::new(501.0, 0.0, 499.0, 600.0))
        );

        assert!(n.remove(1));
        assert_eq!(n, Node::Leaf(3));
        assert!(!n.remove(3), "o último painel não é removido");
    }

    #[test]
    fn neighbors() {
        // [1 | 2]
        // [1 | 3]
        let mut n = Node::Leaf(1);
        n.split(1, Split::Right, 2);
        n.split(2, Split::Down, 3);
        let rects = n.rects(AREA, 2.0);

        assert_eq!(neighbor(&rects, 1, Direction::Right), Some(2));
        assert_eq!(neighbor(&rects, 3, Direction::Left), Some(1));
        assert_eq!(neighbor(&rects, 3, Direction::Up), Some(2));
        assert_eq!(neighbor(&rects, 2, Direction::Down), Some(3));
        assert_eq!(neighbor(&rects, 1, Direction::Left), None);
        assert_eq!(neighbor(&rects, 2, Direction::Right), None);
    }

    #[test]
    fn neighbor_prefers_aligned_pane() {
        // [1 | 2]
        // [3 | 4]   — de 4 para a esquerda vai para 3, não para 1.
        let mut n = Node::Leaf(1);
        n.split(1, Split::Down, 3);
        n.split(1, Split::Right, 2);
        n.split(3, Split::Right, 4);
        let rects = n.rects(AREA, 2.0);
        assert_eq!(neighbor(&rects, 4, Direction::Left), Some(3));
        assert_eq!(neighbor(&rects, 2, Direction::Down), Some(4));
    }

    #[test]
    fn grid_size_respects_padding() {
        assert_eq!(
            grid_size(Rect::new(0.0, 0.0, 822.0, 352.0), 8.0, 17.0, 6.0, 10.0),
            (100, 20)
        );
        assert_eq!(
            grid_size(Rect::new(0.0, 0.0, 5.0, 5.0), 8.0, 17.0, 6.0, 10.0),
            (1, 1)
        );
    }
}
