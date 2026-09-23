//! Throwaway 60 Hz sim. Plain f32, positions in screen points (origin top-left).

pub const DT: f32 = 1.0 / 60.0;
const PLAYER_SPEED: f32 = 420.0;
pub const PLAYER_SIZE: f32 = 26.0;
const BULLET_SPEED: f32 = 1100.0;
pub const BULLET_R: f32 = 3.5;
const FIRE_EVERY_TICKS: u32 = 4; // 15 shots/s

pub const ENEMY_COUNT: usize = 6;
pub const ENEMY_R: f32 = 16.0;
/// After a hit: flash for FLASH ticks, then hidden until RESPAWN ticks have passed.
pub const ENEMY_FLASH_TICKS: u32 = 9;
const ENEMY_RESPAWN_TICKS: u32 = 60;

const ROLL_TICKS: u32 = 12; // 0.2 s of i-frames
const ROLL_SPEED: f32 = 850.0; // ~170 pt per roll
const ROLL_COOLDOWN_TICKS: u32 = 24; // 0.4 s from roll start

/// Aim assist only considers targets within this half-angle of the stick direction.
const ASSIST_CONE_RAD: f32 = 20.0 * std::f32::consts::PI / 180.0;

#[derive(Clone, Copy, Default)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

impl V2 {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
    pub fn lerp(a: V2, b: V2, t: f32) -> V2 {
        V2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    }
    fn angle(self) -> f32 {
        self.y.atan2(self.x)
    }
    fn from_angle(a: f32) -> V2 {
        V2::new(a.cos(), a.sin())
    }
}

/// Signed shortest angle from `a` to `b`, in (-PI, PI].
fn angle_diff(a: f32, b: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (b - a + PI).rem_euclid(TAU) - PI
}

/// Anything that moves keeps its previous-tick position for render interpolation.
#[derive(Clone, Copy)]
pub struct Mover {
    pub prev: V2,
    pub pos: V2,
    pub vel: V2,
}

impl Mover {
    fn integrate(&mut self) {
        self.prev = self.pos;
        self.pos.x += self.vel.x * DT;
        self.pos.y += self.vel.y * DT;
    }
    /// Reflect off the screen edges.
    fn bounce(&mut self, b: V2, r: f32) {
        if self.pos.x < r || self.pos.x > b.x - r {
            self.vel.x = if self.pos.x < r { self.vel.x.abs() } else { -self.vel.x.abs() };
            self.pos.x = self.pos.x.clamp(r, (b.x - r).max(r));
        }
        if self.pos.y < r || self.pos.y > b.y - r {
            self.vel.y = if self.pos.y < r { self.vel.y.abs() } else { -self.vel.y.abs() };
            self.pos.y = self.pos.y.clamp(r, (b.y - r).max(r));
        }
    }
}

pub struct Enemy {
    pub m: Mover,
    /// Ticks until the next wander direction change.
    turn_ticks: u32,
    /// 0 = alive; otherwise ticks since hit (flashing, then hidden, then respawn).
    pub down_ticks: u32,
}

impl Enemy {
    pub fn alive(&self) -> bool {
        self.down_ticks == 0
    }
}

#[derive(Default, Clone, Copy)]
pub enum Aim {
    #[default]
    None,
    /// Fire along the stick.
    Stick(V2),
    /// Fire along the stick, bent toward the nearest target in the cone by `strength` (0..=1).
    Assist(V2, f32),
    /// Fire at the nearest live target.
    Auto,
}

#[derive(Default, Clone, Copy)]
pub struct SimInput {
    /// Magnitude <= 1.
    pub move_dir: V2,
    pub aim: Aim,
    /// Some(dir) requests a roll this tick; a zero dir means "move direction, else facing".
    pub dodge: Option<V2>,
}

pub struct Sim {
    pub bounds: V2,
    pub player: Mover,
    pub bullets: Vec<Mover>,
    pub stress: Vec<Mover>,
    pub enemies: Vec<Enemy>,
    /// Target the aim logic locked onto last tick (for drawing a marker).
    pub aim_target: Option<usize>,
    pub hits: u32,
    pub shots: u32,
    /// Remaining roll ticks; > 0 means rolling with i-frames.
    pub roll_ticks: u32,
    roll_dir: V2,
    roll_cooldown: u32,
    facing: V2,
    fire_cooldown: u32,
    rng: u32,
}

impl Sim {
    pub fn new(bounds: V2) -> Self {
        let c = V2::new(bounds.x * 0.5, bounds.y * 0.5);
        let mut sim = Self {
            bounds,
            player: Mover { prev: c, pos: c, vel: V2::default() },
            bullets: Vec::new(),
            stress: Vec::new(),
            enemies: Vec::new(),
            aim_target: None,
            hits: 0,
            shots: 0,
            roll_ticks: 0,
            roll_dir: V2::default(),
            roll_cooldown: 0,
            facing: V2::new(1.0, 0.0),
            fire_cooldown: 0,
            rng: 0x1234_5678,
        };
        for _ in 0..ENEMY_COUNT {
            let pos = sim.spawn_point();
            sim.enemies.push(Enemy { m: Mover { prev: pos, pos, vel: V2::default() }, turn_ticks: 0, down_ticks: 0 });
        }
        sim
    }

    fn rand(&mut self) -> f32 {
        // xorshift32
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Random point on screen, preferring one not right on top of the player.
    fn spawn_point(&mut self) -> V2 {
        let b = self.bounds;
        let mut p = V2::default();
        for _ in 0..8 {
            p = V2::new(ENEMY_R + self.rand() * (b.x - 2.0 * ENEMY_R), ENEMY_R + self.rand() * (b.y - 2.0 * ENEMY_R));
            if p.sub(self.player.pos).len() > 150.0 {
                break;
            }
        }
        p
    }

    pub fn roll_ready(&self) -> bool {
        self.roll_cooldown == 0
    }

    pub fn set_stress(&mut self, n: usize) {
        self.stress.truncate(n);
        while self.stress.len() < n {
            let pos = V2::new(self.rand() * self.bounds.x, self.rand() * self.bounds.y);
            let a = self.rand() * std::f32::consts::TAU;
            let s = 60.0 + self.rand() * 240.0;
            let vel = V2::new(a.cos() * s, a.sin() * s);
            self.stress.push(Mover { prev: pos, pos, vel });
        }
    }

    /// Nearest live enemy to the player, optionally restricted to a cone around `dir`.
    fn nearest_target(&self, cone: Option<V2>) -> Option<usize> {
        let o = self.player.pos;
        self.enemies
            .iter()
            .enumerate()
            .filter(|(_, e)| e.alive())
            .filter(|(_, e)| {
                cone.is_none_or(|d| angle_diff(d.angle(), e.m.pos.sub(o).angle()).abs() <= ASSIST_CONE_RAD)
            })
            .min_by(|(_, a), (_, b)| a.m.pos.sub(o).len().total_cmp(&b.m.pos.sub(o).len()))
            .map(|(i, _)| i)
    }

    /// Resolve the aim input into a fire direction (unit), updating `aim_target`.
    fn resolve_aim(&mut self, aim: Aim) -> Option<V2> {
        self.aim_target = None;
        let o = self.player.pos;
        match aim {
            Aim::None => None,
            Aim::Stick(d) => Some(V2::from_angle(d.angle())),
            Aim::Assist(d, strength) => {
                let a = d.angle();
                if strength <= 0.0 {
                    return Some(V2::from_angle(a));
                }
                self.aim_target = self.nearest_target(Some(d));
                let Some(i) = self.aim_target else { return Some(V2::from_angle(a)) };
                let ta = self.enemies[i].m.pos.sub(o).angle();
                Some(V2::from_angle(a + angle_diff(a, ta) * strength))
            }
            Aim::Auto => {
                self.aim_target = self.nearest_target(None);
                // No targets: keep shooting where we were facing.
                Some(match self.aim_target {
                    Some(i) => V2::from_angle(self.enemies[i].m.pos.sub(o).angle()),
                    None => self.facing,
                })
            }
        }
    }

    pub fn step(&mut self, input: SimInput) {
        let b = self.bounds;
        let half = PLAYER_SIZE * 0.5;

        if input.move_dir.len() > 0.2 {
            self.facing = V2::from_angle(input.move_dir.angle());
        }

        // Dodge roll.
        self.roll_cooldown = self.roll_cooldown.saturating_sub(1);
        self.roll_ticks = self.roll_ticks.saturating_sub(1);
        if let Some(d) = input.dodge {
            if self.roll_cooldown == 0 {
                let dir = if d.len() > 0.01 {
                    d
                } else if input.move_dir.len() > 0.2 {
                    input.move_dir
                } else {
                    self.facing
                };
                self.roll_dir = V2::from_angle(dir.angle());
                self.roll_ticks = ROLL_TICKS;
                self.roll_cooldown = ROLL_COOLDOWN_TICKS;
            }
        }
        let rolling = self.roll_ticks > 0;
        let vel = if rolling {
            V2::new(self.roll_dir.x * ROLL_SPEED, self.roll_dir.y * ROLL_SPEED)
        } else {
            V2::new(input.move_dir.x * PLAYER_SPEED, input.move_dir.y * PLAYER_SPEED)
        };

        let p = &mut self.player;
        p.prev = p.pos;
        p.pos.x = (p.pos.x + vel.x * DT).clamp(half, (b.x - half).max(half));
        p.pos.y = (p.pos.y + vel.y * DT).clamp(half, (b.y - half).max(half));
        let origin = p.pos;

        for m in &mut self.bullets {
            m.integrate();
        }
        self.bullets
            .retain(|m| m.pos.x > -20.0 && m.pos.y > -20.0 && m.pos.x < b.x + 20.0 && m.pos.y < b.y + 20.0);

        // No firing mid-roll (Gungeon-style).
        self.fire_cooldown = self.fire_cooldown.saturating_sub(1);
        let fire = self.resolve_aim(input.aim);
        if let Some(d) = fire.filter(|_| !rolling) {
            self.facing = d;
            if self.fire_cooldown == 0 {
                let vel = V2::new(d.x * BULLET_SPEED, d.y * BULLET_SPEED);
                self.bullets.push(Mover { prev: origin, pos: origin, vel });
                self.fire_cooldown = FIRE_EVERY_TICKS;
                self.shots += 1;
            }
        }

        // Enemies: slow wander/strafe, occasional direction change.
        for i in 0..self.enemies.len() {
            let e = &self.enemies[i];
            if !e.alive() {
                let t = e.down_ticks + 1;
                if t >= ENEMY_RESPAWN_TICKS {
                    let pos = self.spawn_point();
                    let e = &mut self.enemies[i];
                    e.m = Mover { prev: pos, pos, vel: V2::default() };
                    e.down_ticks = 0;
                    e.turn_ticks = 0;
                } else {
                    let e = &mut self.enemies[i];
                    e.m.prev = e.m.pos;
                    e.down_ticks = t;
                }
                continue;
            }
            if e.turn_ticks == 0 {
                let a = self.rand() * std::f32::consts::TAU;
                let s = 40.0 + self.rand() * 60.0;
                let turn = 45 + (self.rand() * 75.0) as u32;
                let e = &mut self.enemies[i];
                e.m.vel = V2::new(a.cos() * s, a.sin() * s);
                e.turn_ticks = turn;
            }
            let e = &mut self.enemies[i];
            e.turn_ticks -= 1;
            e.m.integrate();
            e.m.bounce(b, ENEMY_R);
        }

        // Bullet vs enemy.
        let hit_r = ENEMY_R + BULLET_R;
        let enemies = &mut self.enemies;
        let hits = &mut self.hits;
        self.bullets.retain(|m| {
            for e in enemies.iter_mut().filter(|e| e.alive()) {
                if m.pos.sub(e.m.pos).len() < hit_r {
                    e.down_ticks = 1;
                    *hits += 1;
                    return false;
                }
            }
            true
        });

        for m in &mut self.stress {
            m.integrate();
            m.bounce(b, 0.0);
        }
    }
}
