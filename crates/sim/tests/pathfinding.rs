//! Enemy pathfinding: the flow field gets rushers out of pockets that local steering
//! can't.

use sim::room::{Category, PrototypeRoom, Tiles, cell_center};
use sim::{ENEMY_RADIUS, FlowField, PLAYER_RADIUS, Tuning, chase, per_tick};

/// A U of wall (cells 3..=7, 3..=5) opening up, away from the player below it.
const POCKET: PrototypeRoom = PrototypeRoom {
    name: "pocket",
    category: Category::Normal,
    cells: &[
        "############",
        "#..........#",
        "#..........#",
        "#..#...#...#",
        "#..#...#...#",
        "#..#####...#",
        "#..........#",
        "#..........#",
        "#..........#",
        "############",
    ],
    exits: &[],
    base: &[],
    reinforcements: &[],
    events: &[],
    extraction: None,
}
.valid();

#[test]
fn a_rusher_climbs_out_of_a_pocket_to_reach_the_player() {
    let tiles = Tiles {
        room: &POCKET,
        sealed: false,
    };
    // Straight below the rusher, through the pocket's base: steering alone pushes into
    // the base and dithers between the arms.
    let player = cell_center(5, 8);
    let mut pos = cell_center(5, 4);
    let mut side = 0;
    let field = FlowField::toward(tiles, &[player]);
    let speed = per_tick(Tuning::NORMAL.rusher_speed);
    let reach = PLAYER_RADIUS.saturating_add(ENEMY_RADIUS);
    // The way round is about 12 cells = 384 pt: ~154 ticks at 2.5 pt/tick.
    let touched = (0..300).position(|_| {
        pos = chase(tiles, &field, pos, player, speed, &mut side);
        let (dx, dy) = (
            pos.x.saturating_sub(player.x),
            pos.y.saturating_sub(player.y),
        );
        dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)) < reach.saturating_mul(reach)
    });
    assert!(touched.is_some(), "stuck at {pos:?}");
}
