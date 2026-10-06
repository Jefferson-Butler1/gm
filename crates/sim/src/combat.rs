//! Gun, bullets, and the enemies (melee rusher, ranged shooter in two patterns), colliding
//! with the floor's tiles; each enemy is kept in the room it stands in. Systems run in a
//! fixed order over arenas iterated in slot order, so every tie resolves the same way on
//! every machine. Speeds and timings come from the run's [`RunConfig`](crate::RunConfig)
//! (issue #15).

use crate::arena::{Arena, Id};
use crate::config::{RunConfig, Tuning, per_tick};
use crate::path::{FlowField, FlowFields, walk_clear};
use crate::player::{PLAYER_RADIUS, Player, dist_sq, scale};
use crate::rng::Rng;
use crate::room::{cell_center, cell_of};
use crate::ship::{Body, HatchId, Spot, Tiles};
use crate::{Event, Fx, FxVec2, Run, SimState, TickEvents, TickInputs, encounter, trig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type EnemyId = Id<Enemy>;

/// Bullets vanish after 60 ticks = 1 s if they hit nothing.
const BULLET_TICKS: u8 = 60;
/// Hitbox radius.
pub const BULLET_RADIUS: Fx = Fx::from_bits(4 << 32);
/// Bullets leave the gun this far ahead of the player's center.
const MUZZLE: Fx = Fx::from_bits(22 << 32);

/// Enemy bullets vanish after 240 ticks = 4 s (1040 pt at Normal speed) if they hit
/// nothing.
const ENEMY_BULLET_TICKS: u8 = 240;
/// Hitbox radius; bigger than the player's bullets so they read as a threat.
pub const ENEMY_BULLET_RADIUS: Fx = Fx::from_bits(5 << 32);

/// Hitbox radius of every enemy; also its half-extent against tiles.
pub const ENEMY_RADIUS: Fx = Fx::from_bits(13 << 32);
/// Enemy HP is in pistol-damage units: Normal's 5 per hit kills a rusher in 2 hits.
pub const RUSHER_HP: u8 = 10;
/// Ticks after a rusher lands a contact hit before it can land another: 0.5 s.
const CONTACT_COOLDOWN: u8 = 30;

/// 2 hits at Normal damage.
pub const SHOOTER_HP: u8 = 10;
/// 3 hits: the spread shooter is the heavier threat (ETG's Shotgun Kin has 2x the HP).
pub const SPREAD_SHOOTER_HP: u8 = 15;
/// 12 hits at Normal damage: the bridge captain, a placeholder elite.
pub const CAPTAIN_HP: u8 = 60;
/// Angle between neighboring spread pellets: 12 degrees.
const PELLET_SPACING: i16 = 2185;
/// The captain's ring: this many slow pellets all round, an eighth turn apart.
const RING_PELLETS: u8 = 8;
/// Shooter walk speed: 2 pt/tick = 120 pt/s.
const SHOOTER_SPEED: Fx = Fx::from_bits(2 << 32);
/// Shooters back off inside this range of their target...
const SHOOTER_NEAR: Fx = Fx::from_bits(120 << 32);
/// ...close in beyond this one, and strafe in between.
const SHOOTER_FAR: Fx = Fx::from_bits(200 << 32);
/// Spawned shooters wait up to this many extra ticks before their first shot, so a wave
/// doesn't fire in unison.
pub const SHOOTER_STAGGER: u32 = 48;

/// Active enemies' centers are kept this far apart: they touch but never stack.
const ENEMY_SPACING: Fx = Fx::from_bits(ENEMY_RADIUS.to_bits().saturating_mul(2));
/// ...and this far from a living player's: pressed 3 pt into contact reach, so contact
/// still lands but a crowd rings the player instead of piling onto it.
const PLAYER_SPACING: Fx = Fx::from_bits(
    PLAYER_RADIUS
        .to_bits()
        .saturating_add(ENEMY_RADIUS.to_bits())
        .saturating_sub(3 << 32),
);
/// A pit respawn shoves active enemies out to this far from the player's center: two
/// cells, so nothing is in contact reach (27 pt) or a step away when it lands.
const RESPAWN_CLEARANCE: Fx = Fx::from_bits(64 << 32);
/// Separation passes per tick; each pass shrinks what a crowd's pressure leaves over.
const SEPARATION_PASSES: u8 = 8;
const HALF: Fx = Fx::from_bits(1 << 31);
/// Turns in `u16` angle units.
const EIGHTH_TURN: i16 = 8192;
const QUARTER_TURN: u16 = 16384;
const HALF_TURN: u16 = 32768;
/// Line-of-fire checks sample the line this often: under a bullet radius, so no wall
/// corner a bullet would clip slips between samples.
const LINE_STEP: Fx = Fx::from_bits(4 << 32);

/// An enemy hunting without a player in sight has reached where it last saw one once it
/// is within 2 cells of the spot; then it starts to give up (see [`Awareness::Alert`]).
const ARRIVED: Fx = Fx::from_bits(64 << 32);

/// A patrolling enemy wanders among the floor cells up to 6 cells (either axis) from its
/// [`Patrol::home`].
const PATROL_RANGE: u32 = 6;
/// Random cells a patrol tries when picking where to walk next; if none is open and
/// reachable, it stands a while longer.
const PATROL_TRIES: u8 = 4;
/// A patrol walk gives up after 6 s wherever it got to (another enemy in the way, say).
const PATROL_WALK_TICKS: u16 = 360;
/// A patrol stand lasts 0.5 s plus up to 1.5 s more.
const PATROL_STAND_TICKS: u16 = 30;
const PATROL_STAND_EXTRA: u32 = 91;
/// Standing, it sweeps its gaze one way then the other, this many ticks each way, at half
/// its turn rate: 50° at the default 150°/s.
const PATROL_LOOK_TICKS: u16 = 40;
/// A patrol walk ends within this of its goal (or a step's length, if longer).
const PATROL_ARRIVED: Fx = Fx::from_bits(2 << 32);

/// How an enemy enters a fight, which sets its spawn telegraph.
///
/// The telegraph is a warning marker during which the enemy is inert: it doesn't move,
/// hurt, push or get pushed, and it can't be targeted or hit (bullets pass through).
/// Minibosses and bosses get their own arrivals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arrival {
    /// Already in the room when the party walks in (the first wave): no telegraph.
    Prespawn,
    /// A later wave warping into a fight in progress: 60 ticks = 1 s of warning.
    Reinforcement,
    /// The bridge captain, whatever its wave: 90 ticks = 1.5 s of warning, already
    /// hunting when it lands.
    Boss,
}

impl Arrival {
    #[must_use]
    pub const fn telegraph_ticks(self) -> u8 {
        match self {
            Self::Prespawn => 0,
            Self::Reinforcement => 60,
            Self::Boss => 90,
        }
    }
}

/// Ticks from all players dying until restart is accepted: 0.75 s, so a panicked tap
/// doesn't skip the death.
pub const DEATH_TICKS: u32 = 45;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bullet {
    pub pos: FxVec2,
    pub vel: FxVec2,
    pub ticks_left: u8,
}

/// One enemy of any type; [`Behavior`] holds what differs by type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Enemy {
    pub pos: FxVec2,
    pub hp: u8,
    pub arrival: Arrival,
    /// Spawn telegraph left, of `arrival`'s; nonzero = inert.
    pub spawn_ticks: u8,
    /// Which way it is detouring around something in its path: +1 turns clockwise
    /// (toward +y from +x), -1 counter-clockwise, 0 = heading straight.
    pub steer: i8,
    /// Where it looks, `u16` turns (as [`crate::trig`]): the middle of its sight cone
    /// (the run's `sight_half_angle` either side). Hunting, it turns toward its target
    /// at the run's `turn_rate`.
    pub facing: u16,
    pub awareness: Awareness,
    /// Where it wanders while unaware.
    pub patrol: Patrol,
    pub behavior: Behavior,
}

/// An unaware enemy's patrol: stand looking around, walk slowly to a random nearby cell,
/// repeat.
///
/// Walks go at the run's `patrol_speed` to an open floor cell near `home` it can reach,
/// facing the way it walks. Never across a hatch or pit: it paths like a hunter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Patrol {
    /// Where it spawned, or where it last gave up a hunt.
    pub home: FxVec2,
    /// Where it is walking; `None` = standing.
    pub goal: Option<FxVec2>,
    /// Ticks left of the walk or stand. A stand at 0 picks the next walk.
    pub ticks: u16,
}

impl Patrol {
    /// About to pick a first walk from `home`.
    #[must_use]
    pub const fn at(home: FxVec2) -> Self {
        Self {
            home,
            goal: None,
            ticks: 0,
        }
    }
}

/// A random patrol stand length, 0.5 to 2 s.
pub fn stand_ticks(rng: &mut Rng) -> u16 {
    let extra = u16::try_from(rng.below(PATROL_STAND_EXTRA)).unwrap_or(0);
    PATROL_STAND_TICKS.saturating_add(extra)
}

/// Whether an enemy knows the party is there. Room enemies start unaware, so a player
/// has to go find them; reinforcements arrive already hunting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Awareness {
    /// Patrols (see [`Patrol`]), or stands still facing one way when the run's
    /// `patrol_speed` is 0; never fires. Notices a player in plain view inside its sight
    /// cone (at any range), that shoots within `hearing_radius`, or that hits it, and an
    /// ally that is hunting within `alert_radius` in plain view. A hatch banging open or
    /// shut within `hatch_hearing_radius` in its room sets it investigating.
    Unaware,
    /// Heard a hatch at `spot` but doesn't know anyone is there: turns toward it and walks
    /// there at twice the run's `patrol_speed` (looks from where it stands at 0), then
    /// sweeps its gaze there, counting `looking` ticks; at the run's `forget_ticks` it
    /// goes back to its patrol. Notices players as when unaware, which alerts it; never
    /// fires nor alerts allies.
    Investigating { spot: FxVec2, looking: u16 },
    /// Hunting. With a player in sight (in its cone) it fights as usual, turning to keep
    /// it in view (and tracks it at any range); otherwise it heads for `last_seen`,
    /// facing it. `searching` counts ticks spent there without finding anyone, turning in
    /// place to look around; at the run's `forget_ticks` it gives up, unaware, and
    /// patrols from where it stands.
    /// Only an enemy still on the hunt (`searching == 0`) alerts its allies, so a group
    /// that has lost the player can calm down instead of re-alerting each other forever.
    Alert { last_seen: FxVec2, searching: u16 },
}

/// What a shooter fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pattern {
    /// One bullet at the target, every `RunConfig::shooter_interval`.
    Aimed,
    /// The pattern experiment: a fan of slow pellets centered on the target (5, or 7 on
    /// Hard), every `RunConfig::spread_interval`.
    Spread,
    /// The bridge captain's (a placeholder elite): the spread fan plus a ring of
    /// [`RING_PELLETS`] slow pellets all round, every `RunConfig::shooter_interval`.
    Captain,
}

impl Pattern {
    fn interval(self, config: &RunConfig) -> u16 {
        match self {
            Self::Aimed | Self::Captain => config.shooter_interval(),
            Self::Spread => config.spread_interval(),
        }
    }
}

/// Each enemy type's own state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Behavior {
    /// Hunting, chases the player it sees (see [`Awareness`]) and hurts on contact.
    Rusher { contact_cooldown: u8 },
    /// Hunting, keeps its distance from the player it sees, strafing, and shoots at it.
    Shooter {
        pattern: Pattern,
        /// Ticks until the next shot. The run's interval for the pattern restarts it;
        /// its last `shooter_telegraph` ticks are the shot telegraph: standing still,
        /// aiming. The telegraph only starts with a clear line of fire, and once started
        /// always ends in a shot (at wherever the target is by then).
        shot_timer: u16,
        /// Strafe direction, +1 or -1 turns (as [`Enemy::steer`]); flips after each shot
        /// and when blocked.
        strafe: i8,
    },
}

impl Enemy {
    /// An unaware rusher arriving at `pos` as a reinforcement, in its telegraph; spawners
    /// set another [`Arrival`] over it.
    #[must_use]
    pub const fn rusher(pos: FxVec2) -> Self {
        Self {
            pos,
            hp: RUSHER_HP,
            arrival: Arrival::Reinforcement,
            spawn_ticks: Arrival::Reinforcement.telegraph_ticks(),
            steer: 0,
            facing: 0,
            awareness: Awareness::Unaware,
            patrol: Patrol::at(pos),
            behavior: Behavior::Rusher {
                contact_cooldown: 0,
            },
        }
    }

    /// An unaware shooter arriving at `pos`, which fires its first shot a full interval
    /// (for its pattern, under `config`) plus `delay` ticks of hunting after its spawn
    /// telegraph.
    #[must_use]
    pub fn shooter(pos: FxVec2, pattern: Pattern, config: &RunConfig, delay: u16) -> Self {
        Self {
            pos,
            hp: match pattern {
                Pattern::Aimed => SHOOTER_HP,
                Pattern::Spread => SPREAD_SHOOTER_HP,
                Pattern::Captain => CAPTAIN_HP,
            },
            arrival: Arrival::Reinforcement,
            spawn_ticks: Arrival::Reinforcement.telegraph_ticks(),
            steer: 0,
            facing: 0,
            awareness: Awareness::Unaware,
            patrol: Patrol::at(pos),
            behavior: Behavior::Shooter {
                pattern,
                shot_timer: pattern.interval(config).saturating_add(delay),
                strafe: 1,
            },
        }
    }

    /// Past the spawn telegraph: moves, hurts, and can be targeted and hit.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.spawn_ticks == 0
    }

    /// Starts (or refreshes) a hunt for a player at `at`. Reports `EnemyAlerted` when it
    /// wasn't hunting.
    fn alert(&mut self, id: EnemyId, at: FxVec2, events: &mut TickEvents) {
        if !self.hunting() {
            events.events.push(Event::EnemyAlerted { enemy: id });
        }
        self.awareness = Awareness::Alert {
            last_seen: at,
            searching: 0,
        };
    }

    /// Sends an unaware enemy to investigate a noise at `spot`. Reports
    /// `EnemyInvestigating` once it is active: a telegraphing spawn reports it as its
    /// telegraph ends.
    pub(crate) fn investigate(&mut self, id: EnemyId, spot: FxVec2, events: &mut TickEvents) {
        if self.awareness != Awareness::Unaware {
            return;
        }
        self.awareness = Awareness::Investigating { spot, looking: 0 };
        if self.active() {
            events.events.push(Event::EnemyInvestigating { enemy: id });
        }
    }

    /// On the hunt (see [`Awareness::Alert`]).
    #[must_use]
    pub const fn hunting(&self) -> bool {
        matches!(self.awareness, Awareness::Alert { .. })
    }

    /// Shot telegraph left, `telegraph..=1` (it fires at 0); `None` when not aiming.
    /// `telegraph` is the run's `shooter_telegraph`.
    #[must_use]
    pub const fn aiming(&self, telegraph: u16) -> Option<u16> {
        match self.behavior {
            Behavior::Shooter { shot_timer, .. } if shot_timer <= telegraph => Some(shot_timer),
            Behavior::Shooter { .. } | Behavior::Rusher { .. } => None,
        }
    }
}

/// One live tick, in order: players (move, fall, respawn, fire), their bullets, enemies
/// (notice, move, fire, separate, contact, telegraph countdown), enemy bullets, the death
/// check, the access panels shot, then the room (waves, hatches, chests, the airlocks).
pub fn tick(state: &mut SimState, inputs: &TickInputs, events: &mut TickEvents) {
    // Hatches only change after everything that moves: panels shot, then
    // `encounter::tick`.
    let (ship, hatches) = (Arc::clone(&state.ship), state.hatches.clone());
    let tiles = Tiles::new(&ship, &hatches);
    // Who alerts allies this tick is decided by last tick's states, so an alert spreads
    // one hop per tick whatever order enemies update in.
    let hunters: Vec<(FxVec2, FxVec2)> = state
        .enemies
        .iter()
        .filter(|(_, e)| e.active())
        .filter_map(|(_, e)| match e.awareness {
            Awareness::Alert {
                last_seen,
                searching: 0,
            } => Some((e.pos, last_seen)),
            Awareness::Alert { .. } | Awareness::Unaware | Awareness::Investigating { .. } => None,
        })
        .collect();
    let shots = players(state, inputs, tiles, events);
    let struck = bullets(state, tiles, events);
    let senses = Senses {
        tiles,
        tuning: state.config.tuning,
        players: targetable(state),
        shots,
        hunters,
    };
    enemies(state, &senses, events);
    enemy_bullets(state, tiles, events);
    if !state.players.iter().flatten().any(Player::alive) {
        state.run = Run::Dead {
            ticks_until_restart: DEATH_TICKS,
        };
        return;
    }
    for hatch in struck {
        encounter::reveal(state, hatch);
    }
    encounter::tick(state, events);
}

/// Moves and fires the players. Returns where each shot this tick was fired from.
fn players(
    state: &mut SimState,
    inputs: &TickInputs,
    tiles: Tiles<'_>,
    events: &mut TickEvents,
) -> Vec<FxVec2> {
    let tuning = state.config.tuning;
    let mut shots = Vec::new();
    let targets: Vec<FxVec2> = state
        .enemies
        .iter()
        .filter(|(_, e)| e.active())
        .map(|(_, e)| e.pos)
        .collect();
    for (slot, (player, input)) in state.players.iter_mut().zip(&inputs.players).enumerate() {
        let Some(player) = player.as_mut().filter(|p| p.alive()) else {
            continue;
        };
        let was_falling = player.falling();
        let shot = player.update(*input, &targets, tiles, &tuning);
        if was_falling && !player.falling() {
            clear_respawn(&mut state.enemies, player.pos, tiles);
        }
        // A fall starts at the full `fall_ticks` (at least 1), and only ever this way.
        if player.fall_ticks == tuning.fall_ticks {
            events.events.push(Event::PlayerFell { slot });
            if !player.alive() {
                events.events.push(Event::PlayerDied { slot });
            }
        }
        if let Some(angle) = shot {
            let dir = trig::unit(angle);
            state.bullets.insert(Bullet {
                pos: add(player.pos, scale(dir, MUZZLE)),
                vel: scale(dir, per_tick(tuning.bullet_speed)),
                ticks_left: BULLET_TICKS,
            });
            events.events.push(Event::ShotFired { slot });
            shots.push(player.pos);
        }
    }
    shots
}

/// Pushes every active enemy within [`RESPAWN_CLEARANCE`] of a pit respawn at `at`
/// straight out to that distance, sliding through `tiles` in quarter steps (each under a
/// cell, as `slide` needs), so one pinned against a wall, pit or its room's edge stays
/// short of it. The post-hit invulnerability the respawn grants covers whatever is left
/// close.
fn clear_respawn(enemies: &mut Arena<Enemy>, at: FxVec2, tiles: Tiles<'_>) {
    const QUARTER: Fx = Fx::from_bits(1 << 30);
    for (_, enemy) in enemies.iter_mut().filter(|(_, e)| e.active()) {
        let tiles = tiles.walker_at(enemy.pos);
        let step = scale(push_out(at, enemy.pos, RESPAWN_CLEARANCE), QUARTER);
        for _ in 0..4 {
            enemy.pos = tiles.slide(enemy.pos, ENEMY_RADIUS, step, Body::Walker);
        }
    }
}

/// Moves a bullet one tick. Returns whether it is still flying: lifetime left, and not
/// in a wall, void or shut hatch (it flies over pits).
fn fly(bullet: &mut Bullet, tiles: Tiles<'_>) -> bool {
    bullet.pos = add(bullet.pos, bullet.vel);
    bullet.ticks_left = bullet.ticks_left.saturating_sub(1);
    bullet.ticks_left > 0 && !tiles.blocks_point(bullet.pos, Body::Shot)
}

/// Returns the hatches bullets stopped in: shot access panels open (see
/// [`encounter::reveal`]).
fn bullets(state: &mut SimState, tiles: Tiles<'_>, events: &mut TickEvents) -> Vec<HatchId> {
    // Each bullet hits at most the first live enemy it overlaps, in slot order. A hit
    // alerts it to the nearest living player (bullets don't record who fired them).
    let damage = state.config.tuning.damage;
    let players: Vec<FxVec2> = state
        .players
        .iter()
        .flatten()
        .filter(|p| p.alive())
        .map(|p| p.pos)
        .collect();
    let enemies = &mut state.enemies;
    let mut struck = Vec::new();
    state.bullets.retain(|_, bullet| {
        if !fly(bullet, tiles) {
            if let Spot::Hatch(hatch) = tiles
                .ship
                .spot(cell_of(bullet.pos.x), cell_of(bullet.pos.y))
            {
                struck.push(hatch);
            }
            return false;
        }
        let reach = BULLET_RADIUS.saturating_add(ENEMY_RADIUS);
        let Some((enemy, e)) = enemies
            .iter_mut()
            .find(|(_, e)| e.hp > 0 && e.active() && overlaps(bullet.pos, e.pos, reach))
        else {
            return true;
        };
        e.hp = e.hp.saturating_sub(damage);
        events.events.push(Event::EnemyHit { enemy });
        if e.hp == 0 {
            events.events.push(Event::EnemyKilled { enemy, pos: e.pos });
        } else if let Some(&shooter) = players.iter().min_by_key(|&&p| dist_sq(p, e.pos)) {
            e.alert(enemy, shooter, events);
        }
        false
    });
    enemies.retain(|_, e| e.hp > 0);
    struck
}

/// Enemy bullets hurt the first player (in slot order) they overlap who can take the hit.
/// Invulnerable players (rolling, or just hurt) don't stop them: dodged bullets fly on.
fn enemy_bullets(state: &mut SimState, tiles: Tiles<'_>, events: &mut TickEvents) {
    let hurt_ticks = state.config.tuning.hurt_ticks;
    let players = &mut state.players;
    state.enemy_bullets.retain(|_, bullet| {
        if !fly(bullet, tiles) {
            return false;
        }
        let reach = ENEMY_BULLET_RADIUS.saturating_add(PLAYER_RADIUS);
        let hit = players.iter_mut().enumerate().find_map(|(slot, player)| {
            let player = player.as_mut()?;
            (overlaps(bullet.pos, player.pos, reach) && player.hurt(hurt_ticks))
                .then_some((slot, player))
        });
        let Some((slot, player)) = hit else {
            return true;
        };
        events.events.push(Event::PlayerHit { slot });
        if !player.alive() {
            events.events.push(Event::PlayerDied { slot });
        }
        false
    });
}

/// What enemies can perceive this tick (see [`notice`]).
struct Senses<'a> {
    tiles: Tiles<'a>,
    tuning: Tuning,
    /// Targetable players' positions.
    players: Vec<FxVec2>,
    /// Where players fired from this tick.
    shots: Vec<FxVec2>,
    /// Enemies on the hunt as of last tick: where each is, and where it's headed.
    hunters: Vec<(FxVec2, FxVec2)>,
}

/// Positions of the players enemies chase, aim at and crowd.
fn targetable(state: &SimState) -> Vec<FxVec2> {
    state
        .players
        .iter()
        .flatten()
        .filter(|p| p.targetable())
        .map(|p| p.pos)
        .collect()
}

/// Updates `enemy`'s awareness from what it perceives, and returns the player it sees:
/// the nearest in plain view inside its sight cone, at any range.
/// Failing that, a shot it hears (fired in its own room: hatches stop sound) or an ally
/// hunting in plain view alerts it.
fn notice(
    enemy: &mut Enemy,
    id: EnemyId,
    senses: &Senses<'_>,
    events: &mut TickEvents,
) -> Option<FxVec2> {
    let tuning = &senses.tuning;
    let pos = enemy.pos;
    let within = |p: FxVec2, radius: u16| overlaps(p, pos, Fx::from_num(radius));
    let nearest = |p: &FxVec2| dist_sq(*p, pos);
    let hunting = enemy.hunting();
    let (facing, half) = (enemy.facing, tuning.sight_half_turns());
    let in_cone = |p: FxVec2| {
        // Standing on the enemy has no direction: that counts as seen.
        trig::angle_of(sub(p, pos))
            .is_none_or(|to| trig::angle_diff(facing, to).unsigned_abs() <= half)
    };
    let seen = senses
        .players
        .iter()
        .copied()
        .filter(|&p| in_cone(p) && line_of_fire(senses.tiles, pos, p))
        .min_by_key(nearest);
    let ship = senses.tiles.ship;
    let heard = || {
        senses
            .shots
            .iter()
            .copied()
            .filter(|&p| within(p, tuning.hearing_radius) && ship.room_at(p) == ship.room_at(pos))
            .min_by_key(nearest)
    };
    let told = || {
        senses
            .hunters
            .iter()
            .filter(|&&(ally, _)| {
                !hunting
                    && within(ally, tuning.alert_radius)
                    && line_of_fire(senses.tiles, ally, pos)
            })
            .min_by_key(|(ally, _)| nearest(ally))
            .map(|&(_, last_seen)| last_seen)
    };
    if let Some(at) = seen.or_else(heard).or_else(told) {
        enemy.alert(id, at, events);
    }
    seen
}

/// Where `enemy` heads this tick: the player it sees, else the spot it last saw one,
/// where it searches for `forget_ticks` before giving up, unaware, to patrol from where
/// it stands. `None` = not hunting: it patrols or investigates.
fn hunt(enemy: &mut Enemy, seen: Option<FxVec2>, forget_ticks: u16) -> Option<FxVec2> {
    let Awareness::Alert {
        last_seen,
        searching,
    } = &mut enemy.awareness
    else {
        return None;
    };
    if seen.is_some() {
        return seen;
    }
    if overlaps(enemy.pos, *last_seen, ARRIVED) {
        *searching = searching.saturating_add(1);
        if *searching >= forget_ticks {
            enemy.awareness = Awareness::Unaware;
            enemy.patrol = Patrol::at(enemy.pos);
            return None;
        }
    }
    Some(*last_seen)
}

/// Turns a hunting `enemy` up to `rate` (`u16` turns): toward `target`, or, while
/// searching where it lost the player, steadily around to look for it.
fn turn(enemy: &mut Enemy, target: FxVec2, rate: u16) {
    if let Awareness::Alert { searching, .. } = enemy.awareness
        && searching > 0
    {
        enemy.facing = enemy.facing.wrapping_add(rate);
        return;
    }
    let Some(to) = trig::angle_of(sub(target, enemy.pos)) else {
        return;
    };
    let rate = i16::try_from(rate).unwrap_or(i16::MAX);
    let step = trig::angle_diff(enemy.facing, to).clamp(rate.saturating_neg(), rate);
    enemy.facing = enemy.facing.wrapping_add_signed(step);
}

fn enemies(state: &mut SimState, senses: &Senses<'_>, events: &mut TickEvents) {
    let config = state.config;
    let telegraph = config.tuning.shooter_telegraph;
    let rusher_speed = per_tick(config.tuning.rusher_speed);
    let turn_rate = config.tuning.turn_per_tick();
    let mut fields = FlowFields::new();
    for (id, enemy) in state.enemies.iter_mut().filter(|(_, e)| e.active()) {
        // With no targetable player (all falling or dead), enemies hold still, timers
        // paused.
        if senses.players.is_empty() {
            continue;
        }
        let seen = notice(enemy, id, senses, events);
        let Some(target) = hunt(enemy, seen, config.tuning.forget_ticks) else {
            off_hunt(enemy, senses, &mut fields, &mut state.rng);
            continue;
        };
        turn(enemy, target, turn_rate);
        // No direction when exactly on the target: stay put (contact still applies).
        let Some(angle) = trig::angle_of(sub(target, enemy.pos)) else {
            continue;
        };
        let tiles = senses.tiles.walker_at(enemy.pos);
        let field = fields.toward(tiles, target);
        let pursue = |pos, speed, side: &mut i8| chase(tiles, field, pos, target, speed, side);
        match &mut enemy.behavior {
            Behavior::Rusher { contact_cooldown } => {
                *contact_cooldown = contact_cooldown.saturating_sub(1);
                enemy.pos = pursue(enemy.pos, rusher_speed, &mut enemy.steer);
            }
            Behavior::Shooter {
                pattern,
                shot_timer,
                strafe,
            } => {
                if *shot_timer <= telegraph {
                    // Aiming: stand still, then fire at wherever the target is now (where
                    // the player was last seen, if it has slipped out of sight).
                    *shot_timer = shot_timer.saturating_sub(1);
                    if *shot_timer == 0 {
                        fire(
                            &mut state.enemy_bullets,
                            enemy.pos,
                            angle,
                            *pattern,
                            &config,
                        );
                        events.events.push(Event::EnemyFired { enemy: id });
                        *shot_timer = pattern.interval(&config);
                        *strafe = strafe.saturating_neg();
                    }
                    continue;
                }
                // Seeing a player is a clear line of fire to it.
                let clear = seen.is_some();
                // Only start aiming with a clear line of fire; otherwise keep repositioning.
                if *shot_timer > telegraph.saturating_add(1) || clear {
                    *shot_timer = shot_timer.saturating_sub(1);
                }
                enemy.pos = if !clear || !overlaps(enemy.pos, target, SHOOTER_FAR) {
                    pursue(enemy.pos, SHOOTER_SPEED, &mut enemy.steer)
                } else if overlaps(enemy.pos, target, SHOOTER_NEAR) {
                    let away = angle.wrapping_add(HALF_TURN);
                    steer(tiles, enemy.pos, away, SHOOTER_SPEED, &mut enemy.steer)
                } else {
                    let side = if *strafe < 0 {
                        angle.wrapping_sub(QUARTER_TURN)
                    } else {
                        angle.wrapping_add(QUARTER_TURN)
                    };
                    let to = walk(tiles, enemy.pos, side, SHOOTER_SPEED);
                    if !progressed(enemy.pos, to, SHOOTER_SPEED) {
                        *strafe = strafe.saturating_neg();
                    }
                    to
                };
            }
        }
    }

    separate(state, senses.tiles);
    telegraphs_and_contact(state, events);
}

/// After enemies move: spawn telegraphs count down, and rushers touching a player hurt it.
fn telegraphs_and_contact(state: &mut SimState, events: &mut TickEvents) {
    let config = state.config;
    let reach = PLAYER_RADIUS.saturating_add(ENEMY_RADIUS);
    for (id, enemy) in state.enemies.iter_mut() {
        if !enemy.active() {
            // Counted down last, so a telegraph of N ticks is inert for exactly N.
            enemy.spawn_ticks = enemy.spawn_ticks.saturating_sub(1);
            // A noise heard while telegraphing shows once it can act on it.
            if enemy.active() && matches!(enemy.awareness, Awareness::Investigating { .. }) {
                events.events.push(Event::EnemyInvestigating { enemy: id });
            }
            continue;
        }
        let Behavior::Rusher { contact_cooldown } = &mut enemy.behavior else {
            continue;
        };
        if *contact_cooldown > 0 {
            continue;
        }
        for (slot, player) in state.players.iter_mut().enumerate() {
            if let Some(player) = player
                && overlaps(player.pos, enemy.pos, reach)
                && player.hurt(config.tuning.hurt_ticks)
            {
                *contact_cooldown = CONTACT_COOLDOWN;
                events.events.push(Event::PlayerHit { slot });
                if !player.alive() {
                    events.events.push(Event::PlayerDied { slot });
                }
                break;
            }
        }
    }
}

/// One tick of an `enemy` not hunting: investigating a noise, else on patrol.
fn off_hunt<'a>(
    enemy: &mut Enemy,
    senses: &Senses<'a>,
    fields: &mut FlowFields<'a>,
    rng: &mut Rng,
) {
    match enemy.awareness {
        Awareness::Investigating { spot, looking } => {
            investigate(enemy, spot, looking, senses, fields);
        }
        Awareness::Unaware | Awareness::Alert { .. } => patrol(enemy, senses, fields, rng),
    }
}

/// One tick of an unaware `enemy`'s [`Patrol`] at the run's `patrol_speed` (none at 0).
/// Walking, it turns (at the run's `turn_rate`) to face its heading first, stepping only
/// once that is within an eighth turn.
fn patrol<'a>(enemy: &mut Enemy, senses: &Senses<'a>, fields: &mut FlowFields<'a>, rng: &mut Rng) {
    let speed = per_tick(senses.tuning.patrol_speed);
    if speed == Fx::ZERO {
        return;
    }
    let (tiles, rate) = (
        senses.tiles.walker_at(enemy.pos),
        senses.tuning.turn_per_tick(),
    );
    let mut p = enemy.patrol;
    p.ticks = p.ticks.saturating_sub(1);
    match p.goal {
        None if p.ticks > 0 => look_around(enemy, p.ticks, rate),
        None => {
            p.goal = pick_goal(enemy.pos, p.home, tiles, fields, rng);
            p.ticks = match p.goal {
                Some(_) => PATROL_WALK_TICKS,
                None => stand_ticks(rng),
            };
        }
        Some(goal) if p.ticks == 0 || overlaps(enemy.pos, goal, speed.max(PATROL_ARRIVED)) => {
            p.goal = None;
            p.ticks = stand_ticks(rng);
        }
        Some(goal) => stroll(enemy, goal, speed, tiles, fields, rate),
    }
    enemy.patrol = p;
}

/// One tick of an [`Awareness::Investigating`] `enemy`: walk to `spot`, then look around
/// there until the run's `forget_ticks`, then back to its patrol (from its stand, so it
/// picks a walk near its home next).
fn investigate<'a>(
    enemy: &mut Enemy,
    spot: FxVec2,
    looking: u16,
    senses: &Senses<'a>,
    fields: &mut FlowFields<'a>,
) {
    let tuning = &senses.tuning;
    let (tiles, rate) = (senses.tiles.walker_at(enemy.pos), tuning.turn_per_tick());
    let speed = per_tick(tuning.patrol_speed.saturating_mul(2));
    if looking == 0 && speed > Fx::ZERO && !overlaps(enemy.pos, spot, ARRIVED) {
        stroll(enemy, spot, speed, tiles, fields, rate);
        return;
    }
    let looking = looking.saturating_add(1);
    if looking >= tuning.forget_ticks {
        enemy.awareness = Awareness::Unaware;
        enemy.patrol.goal = None;
        enemy.patrol.ticks = 0;
        return;
    }
    enemy.awareness = Awareness::Investigating { spot, looking };
    look_around(enemy, looking, rate);
}

/// Standing, sweeps `enemy`'s gaze one way then the other, [`PATROL_LOOK_TICKS`] each
/// way by `ticks` (a counter), at half its turn rate `rate`.
const fn look_around(enemy: &mut Enemy, ticks: u16, rate: u16) {
    let look = rate / 2;
    enemy.facing = if (ticks / PATROL_LOOK_TICKS).is_multiple_of(2) {
        enemy.facing.wrapping_add(look)
    } else {
        enemy.facing.wrapping_sub(look)
    };
}

/// One unhurried step of `enemy` toward `goal` at `speed` through `tiles` (its room):
/// it turns (at `rate`) to face its heading first, stepping only once that is within an
/// eighth turn.
fn stroll<'a>(
    enemy: &mut Enemy,
    goal: FxVec2,
    speed: Fx,
    tiles: Tiles<'a>,
    fields: &mut FlowFields<'a>,
    rate: u16,
) {
    let field = fields.toward(tiles, goal);
    let to = chase(tiles, field, enemy.pos, goal, speed, &mut enemy.steer);
    turn(enemy, to, rate);
    let facing = enemy.facing;
    let ahead = trig::angle_of(sub(to, enemy.pos)).is_some_and(|heading| {
        trig::angle_diff(facing, heading).unsigned_abs() <= EIGHTH_TURN.unsigned_abs()
    });
    if ahead {
        enemy.pos = to;
    }
}

/// A random open floor cell's center within [`PATROL_RANGE`] of `home`'s cell that a
/// walker at `pos` can reach through `tiles` (its room), other than the one it is in.
/// `None` if [`PATROL_TRIES`] draws find none.
fn pick_goal<'a>(
    pos: FxVec2,
    home: FxVec2,
    tiles: Tiles<'a>,
    fields: &mut FlowFields<'a>,
    rng: &mut Rng,
) -> Option<FxVec2> {
    let span = PATROL_RANGE.saturating_mul(2).saturating_add(1);
    let range = i32::try_from(PATROL_RANGE).unwrap_or(0);
    let mut offset = || i32::try_from(rng.below(span)).map_or(0, |d| d.saturating_sub(range));
    let here = (cell_of(pos.x), cell_of(pos.y));
    (0..PATROL_TRIES).find_map(|_| {
        let x = cell_of(home.x).saturating_add(offset());
        let y = cell_of(home.y).saturating_add(offset());
        if (x, y) == here || tiles.blocks(x, y, Body::Walker) {
            return None;
        }
        let goal = cell_center(usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        let reachable = walk_clear(tiles, pos, goal, ENEMY_RADIUS)
            || fields.toward(tiles, goal).next(pos).is_some();
        reachable.then_some(goal)
    })
}

/// A shooter at `from` fires its `pattern` centered on `angle`.
fn fire(
    bullets: &mut Arena<Bullet>,
    from: FxVec2,
    angle: u16,
    pattern: Pattern,
    config: &RunConfig,
) {
    let pellets = (config.difficulty.spread_pellets(), PELLET_SPACING);
    let (count, spacing) = match pattern {
        Pattern::Aimed => (1, 0),
        Pattern::Spread | Pattern::Captain => pellets,
    };
    let speed = match pattern {
        Pattern::Aimed => config.enemy_bullet_speed(),
        Pattern::Spread | Pattern::Captain => config.pellet_speed(),
    };
    volley(bullets, from, angle, (count, spacing), speed);
    if pattern == Pattern::Captain {
        // Offset half a gap, so no ring pellet doubles the fan's middle one.
        let ring = angle.wrapping_add_signed(EIGHTH_TURN / 2);
        volley(bullets, from, ring, (RING_PELLETS, EIGHTH_TURN), speed);
    }
}

/// `count` bullets from `from` at `speed`, `spacing` apart and centered on `angle`.
fn volley(
    bullets: &mut Arena<Bullet>,
    from: FxVec2,
    angle: u16,
    (count, spacing): (u8, i16),
    speed: u16,
) {
    for i in 0..count {
        // Offsets from the center: (2i - (count - 1)) half-spacings.
        let halves = i16::from(i)
            .saturating_mul(2)
            .saturating_sub(i16::from(count).saturating_sub(1));
        let offset = halves.saturating_mul(spacing / 2);
        let dir = trig::unit(angle.wrapping_add_signed(offset));
        bullets.insert(Bullet {
            pos: add(from, scale(dir, ENEMY_RADIUS)),
            vel: scale(dir, per_tick(speed)),
            ticks_left: ENEMY_BULLET_TICKS,
        });
    }
}

/// One enemy step through the tiles toward `angle`.
fn walk(tiles: Tiles<'_>, pos: FxVec2, angle: u16, speed: Fx) -> FxVec2 {
    tiles.slide(
        pos,
        ENEMY_RADIUS,
        scale(trig::unit(angle), speed),
        Body::Walker,
    )
}

/// Whether a step of `speed` covered at least half of that.
fn progressed(from: FxVec2, to: FxVec2, speed: Fx) -> bool {
    let half = i128::from(speed.to_bits()).checked_div(2).unwrap_or(0);
    dist_sq(from, to) >= half.saturating_mul(half)
}

/// One step of an enemy at `pos` closing in on `target`.
///
/// Straight at it while its body fits the whole way, else toward the next cell along
/// `field` (around walls, pits and pockets; `field` may lead elsewhere when it was built
/// toward several goals). Either way [`steer`] detours around what the heading clips.
/// `side` is [`Enemy::steer`].
#[must_use]
pub fn chase(
    tiles: Tiles<'_>,
    field: &FlowField,
    pos: FxVec2,
    target: FxVec2,
    speed: Fx,
    side: &mut i8,
) -> FxVec2 {
    let toward = if walk_clear(tiles, pos, target, ENEMY_RADIUS) {
        target
    } else {
        field.next(pos).unwrap_or(target)
    };
    let Some(angle) = trig::angle_of(sub(toward, pos)) else {
        return pos;
    };
    steer(tiles, pos, angle, speed, side)
}

/// A step toward `angle` that detours around whatever blocks it, locally ([`chase`]
/// picks the heading, from the flow field when the way isn't clear). Walking already
/// slides along walls, so this only matters when the way is blocked nearly head-on:
/// then it tries 45° and 90° off to one side, then the other. The side
/// that works is kept in `side` while the direct way stays blocked, so a body follows a
/// pillar's face around its corner instead of dithering. The first side tried is the one
/// the blocked step drifted toward.
fn steer(tiles: Tiles<'_>, pos: FxVec2, angle: u16, speed: Fx, side: &mut i8) -> FxVec2 {
    let direct = walk(tiles, pos, angle, speed);
    if progressed(pos, direct, speed) {
        *side = 0;
        return direct;
    }
    let first = if *side == 0 {
        // The cross product's sign: which side of the heading the blocked step slid to.
        let (dir, drift) = (trig::unit(angle), sub(direct, pos));
        let cross = dir
            .x
            .saturating_mul(drift.y)
            .saturating_sub(dir.y.saturating_mul(drift.x));
        if cross < Fx::ZERO { -1 } else { 1 }
    } else {
        *side
    };
    for s in [first, first.saturating_neg()] {
        for turns in [1, 2] {
            let turn = EIGHTH_TURN
                .saturating_mul(turns)
                .saturating_mul(i16::from(s));
            let to = walk(tiles, pos, angle.wrapping_add_signed(turn), speed);
            if progressed(pos, to, speed) {
                *side = s;
                return to;
            }
        }
    }
    direct
}

/// Whether a shot from `from` to `to` would clear every wall, void and shut hatch on the
/// way (pits don't block shots).
fn line_of_fire(tiles: Tiles<'_>, from: FxVec2, to: FxVec2) -> bool {
    let offset = sub(to, from);
    let steps = dist(from, to)
        .checked_div(LINE_STEP)
        .map_or(1, |n| n.saturating_to_num::<i64>().max(1));
    let (Some(dx), Some(dy)) = (
        offset.x.checked_div_int(steps),
        offset.y.checked_div_int(steps),
    ) else {
        return false;
    };
    let mut at = from;
    (0..steps).all(|_| {
        at = add(at, FxVec2 { x: dx, y: dy });
        !tiles.blocks_point(at, Body::Shot)
    })
}

/// Resolves overlaps among active enemies: pushes them apart and out of targetable
/// players.
/// Every push slides through `tiles` like a walk, so nobody is shoved into a wall, pit,
/// void, hatch or another room; a body pinned against one leaves the rest of the
/// correction to later passes and to its neighbors. Pairs resolve in slot order, each
/// pass building on the last, so the result is deterministic, and a crowd pressing on a
/// player settles into a still ring around it instead of stacking.
fn separate(state: &mut SimState, tiles: Tiles<'_>) {
    let players = targetable(state);
    // Each body with the view that keeps it in its room.
    let mut bodies: Vec<(FxVec2, Tiles<'_>)> = state
        .enemies
        .iter()
        .filter(|(_, e)| e.active())
        .map(|(_, e)| (e.pos, tiles.walker_at(e.pos)))
        .collect();
    // Pushes stay under a cell, as `slide` needs: a pair's half is at most
    // ENEMY_SPACING / 2 and a player's push at most PLAYER_SPACING.
    let slide = |(pos, tiles): &mut (FxVec2, Tiles<'_>), push: FxVec2| {
        *pos = tiles.slide(*pos, ENEMY_RADIUS, push, Body::Walker);
    };
    for _ in 0..SEPARATION_PASSES {
        let mut rest = bodies.as_mut_slice();
        while let Some((a, tail)) = rest.split_first_mut() {
            for b in tail.iter_mut() {
                // Each side of the pair takes half the correction.
                let push = scale(push_out(a.0, b.0, ENEMY_SPACING), HALF);
                slide(a, sub(FxVec2::default(), push));
                slide(b, push);
            }
            rest = tail;
        }
        for body in &mut bodies {
            // Players don't budge.
            for &player in &players {
                let push = push_out(player, body.0, PLAYER_SPACING);
                slide(body, push);
            }
        }
    }
    let active = state.enemies.iter_mut().filter(|(_, e)| e.active());
    for ((_, enemy), (body, _)) in active.zip(bodies) {
        enemy.pos = body;
    }
}

/// How far `b` must move directly away from `a` for their centers to be `spacing`
/// apart; zero if they already are. Coincident centers part along +x.
fn push_out(a: FxVec2, b: FxVec2, spacing: Fx) -> FxVec2 {
    let spacing_bits = i128::from(spacing.to_bits());
    if dist_sq(a, b) >= spacing_bits.saturating_mul(spacing_bits) {
        return FxVec2::default();
    }
    let gap = dist(a, b);
    let offset = sub(b, a);
    let dir = match (offset.x.checked_div(gap), offset.y.checked_div(gap)) {
        (Some(x), Some(y)) => FxVec2 { x, y },
        _ => FxVec2 {
            x: Fx::ONE,
            y: Fx::ZERO,
        },
    };
    scale(dir, spacing.saturating_sub(gap))
}

/// Distance between two points.
fn dist(a: FxVec2, b: FxVec2) -> Fx {
    // `dist_sq` is in squared raw bits, so its root is in raw bits.
    Fx::from_bits(i64::try_from(dist_sq(a, b).unsigned_abs().isqrt()).unwrap_or(i64::MAX))
}

/// Circles whose radii sum to `reach` overlap.
fn overlaps(a: FxVec2, b: FxVec2, reach: Fx) -> bool {
    let reach = i128::from(reach.to_bits());
    dist_sq(a, b) < reach.saturating_mul(reach)
}

const fn add(a: FxVec2, b: FxVec2) -> FxVec2 {
    FxVec2 {
        x: a.x.saturating_add(b.x),
        y: a.y.saturating_add(b.y),
    }
}

const fn sub(a: FxVec2, b: FxVec2) -> FxVec2 {
    FxVec2 {
        x: a.x.saturating_sub(b.x),
        y: a.y.saturating_sub(b.y),
    }
}
