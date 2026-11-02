use super::*;
use crate::layout::{MAX_DEPTH, Rect};

fn depth(node: &Node) -> usize {
    match node {
        Node::Leaf(_) => 1,
        Node::Split { ratio, a, b, .. } => {
            assert!(ratio.is_finite() && *ratio > 0.0 && *ratio < 1.0);
            assert!((ratio + (1.0 - ratio) - 1.0).abs() < f32::EPSILON);
            1 + depth(a).max(depth(b))
        }
    }
}

#[test]
fn every_preset_preserves_one_to_six_leaves_and_nonzero_small_geometry() {
    for preset in Preset::ALL {
        assert!(preset.build(&[]).is_none());
        for count in 1..=6 {
            let ids: Vec<_> = (0..count).map(|i| 31 - i * 2).collect();
            let node = preset.build(&ids).unwrap();
            let mut leaves = vec![];
            node.leaves(&mut leaves);
            assert_eq!(leaves, ids);
            assert!(depth(&node) <= MAX_DEPTH);
            for (w, h) in [(40, 10), (40, 7), (120, 30)] {
                let mut cells = vec![];
                node.layout(Rect { x: 3, y: 2, w, h }, &mut cells);
                assert_eq!(cells.len(), count);
                for (i, (_, r)) in cells.iter().enumerate() {
                    assert!(r.w > 0 && r.h > 0, "{preset:?}/{count}/{w}x{h}: {cells:?}");
                    assert!(r.x >= 3 && r.y >= 2 && r.x + r.w <= 3 + w && r.y + r.h <= 2 + h);
                    for (_, other) in &cells[..i] {
                        assert!(
                            r.x + r.w <= other.x
                                || other.x + other.w <= r.x
                                || r.y + r.h <= other.y
                                || other.y + other.h <= r.y
                        );
                    }
                }
                if matches!(preset, Preset::EvenHorizontal | Preset::EvenVertical) {
                    let extents: Vec<_> = cells
                        .iter()
                        .map(|(_, r)| {
                            if preset == Preset::EvenHorizontal {
                                r.w
                            } else {
                                r.h
                            }
                        })
                        .collect();
                    assert!(extents.iter().max().unwrap() - extents.iter().min().unwrap() <= 1);
                }
            }
        }
    }
}

#[test]
fn named_main_and_tiled_layouts_have_the_documented_axes_and_cycle() {
    let ids = [1, 2, 3, 4, 5, 6];
    for (preset, axis) in [
        (Preset::MainVertical, Dir::H),
        (Preset::MainHorizontal, Dir::V),
    ] {
        let Node::Split { dir, ratio, a, .. } = preset.build(&ids).unwrap() else {
            panic!("split");
        };
        assert_eq!(dir, axis);
        assert_eq!(ratio, 0.6);
        assert_eq!(*a, Node::Leaf(1));
    }
    let mut rects = vec![];
    Preset::Tiled.build(&ids).unwrap().layout(
        Rect {
            x: 0,
            y: 0,
            w: 80,
            h: 24,
        },
        &mut rects,
    );
    assert_eq!(rects.iter().filter(|(_, r)| r.y == 0).count(), 3);
    for (i, preset) in Preset::ALL.into_iter().enumerate() {
        assert_eq!(preset.name().parse::<Preset>(), Ok(preset));
        assert_eq!(preset.next(), Preset::ALL[(i + 1) % 5]);
    }
    assert!("unknown".parse::<Preset>().is_err());
}

#[test]
fn presets_remain_bounded_for_large_windows() {
    let ids: Vec<_> = (1..=4096).collect();
    for preset in Preset::ALL {
        assert!(depth(&preset.build(&ids).unwrap()) <= MAX_DEPTH);
    }
}
