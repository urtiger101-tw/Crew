//! tmux's five named layouts. Rebuilding uses existing leaves, never pane objects.
use super::{Dir, Node};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

impl Preset {
    /// tmux's M-1..M-5 and next-layout order.
    pub const ALL: [Self; 5] = [
        Self::EvenHorizontal,
        Self::EvenVertical,
        Self::MainHorizontal,
        Self::MainVertical,
        Self::Tiled,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::EvenHorizontal => "even-horizontal",
            Self::EvenVertical => "even-vertical",
            Self::MainHorizontal => "main-horizontal",
            Self::MainVertical => "main-vertical",
            Self::Tiled => "tiled",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::EvenHorizontal => Self::EvenVertical,
            Self::EvenVertical => Self::MainHorizontal,
            Self::MainHorizontal => Self::MainVertical,
            Self::MainVertical => Self::Tiled,
            Self::Tiled => Self::EvenHorizontal,
        }
    }

    /// Balanced binary strips keep depth logarithmic rather than one per pane.
    pub fn build(self, ids: &[usize]) -> Option<Node> {
        let first = *ids.first()?;
        if ids.len() == 1 {
            return Some(Node::Leaf(first));
        }
        let leaves: Vec<_> = ids.iter().copied().map(Node::Leaf).collect();
        Some(match self {
            Self::EvenHorizontal => strip(&leaves, Dir::H),
            Self::EvenVertical => strip(&leaves, Dir::V),
            Self::MainHorizontal | Self::MainVertical => {
                let dir = if self == Self::MainVertical {
                    Dir::H
                } else {
                    Dir::V
                };
                let secondary = if dir == Dir::H { Dir::V } else { Dir::H };
                Node::Split {
                    dir,
                    ratio: 0.6,
                    a: Box::new(Node::Leaf(first)),
                    b: Box::new(strip(&leaves[1..], secondary)),
                }
            }
            Self::Tiled => {
                let columns = ids.len().isqrt() + usize::from(ids.len().isqrt().pow(2) < ids.len());
                let rows = ids.len().div_ceil(columns);
                let mut offset = 0;
                let mut row_nodes = vec![];
                for row in 0..rows {
                    let count = ids.len() / rows + usize::from(row < ids.len() % rows);
                    row_nodes.push(strip(&leaves[offset..offset + count], Dir::H));
                    offset += count;
                }
                strip(&row_nodes, Dir::V)
            }
        })
    }
}

impl std::str::FromStr for Preset {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL.into_iter().find(|layout| layout.name() == name).ok_or_else(|| format!(
            "unknown layout '{name}'; expected even-horizontal|even-vertical|main-horizontal|main-vertical|tiled"))
    }
}

fn strip(nodes: &[Node], dir: Dir) -> Node {
    if nodes.len() == 1 {
        return nodes[0].clone();
    }
    let middle = nodes.len() / 2;
    Node::Split {
        dir,
        ratio: middle as f32 / nodes.len() as f32,
        a: Box::new(strip(&nodes[..middle], dir)),
        b: Box::new(strip(&nodes[middle..], dir)),
    }
}

#[cfg(test)]
mod tests;
