//! The floor's Scrap budget (issue #57): rolled from the run seed, split across every
//! enemy placement.

use sim::pickup::{CAPTAIN_SCRAP, SCRAP_MEAN, SCRAP_MIN, SCRAP_SPREAD, budget, scrap_total};
use sim::room::EnemyKind;
use sim::{RunConfig, Ship, SimState, Template};

#[test]
fn the_budget_is_seeded_sums_to_the_total_and_pays_captains_their_share() {
    let (low, high) = (
        SCRAP_MEAN.saturating_sub(SCRAP_SPREAD).max(SCRAP_MIN),
        SCRAP_MEAN.saturating_add(SCRAP_SPREAD),
    );
    let mut totals = Vec::new();
    for seed in 0..64 {
        let ship = Ship::generate(Template::for_seed(seed), seed);
        let shares = budget(seed, &ship);
        assert_eq!(shares, budget(seed, &ship), "seed {seed}: deterministic");
        assert_eq!(
            SimState::new(seed, RunConfig::default()).scrap_shares,
            shares,
            "seed {seed}: the run rolls it with its ship"
        );
        let total = scrap_total(seed);
        assert!((low..=high).contains(&total), "seed {seed}: {total}");
        let sum: u16 = shares.iter().copied().map(u16::from).sum();
        assert_eq!(sum, total, "seed {seed}: {shares:?}");

        let kinds: Vec<EnemyKind> = (ship.rooms().iter())
            .flat_map(|p| {
                let layers = p.room.reinforcements.iter().map(|r| r.placements);
                std::iter::once(p.room.base).chain(layers)
            })
            .flatten()
            .map(|p| p.kind)
            .collect();
        assert_eq!(shares.len(), kinds.len(), "seed {seed}: one per placement");
        for (share, kind) in shares.iter().zip(&kinds) {
            if *kind == EnemyKind::Captain {
                assert_eq!(*share, CAPTAIN_SCRAP, "seed {seed}");
            }
        }
        totals.push(total);
    }
    totals.dedup();
    assert!(totals.len() > 1, "the total varies by seed");
}
