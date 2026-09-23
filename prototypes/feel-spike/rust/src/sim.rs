//! Throwaway 60 Hz sim. Plain f32, positions in screen points (origin top-left).

pub const DT: f32 = 1.0 / 60.0;
const PLAYER_SPEED: f32 = 420.0;
pub const PLAYER_SIZE: f32 = 26.0;
const BULLET_SPEED: f32 = 1100.0;
const FIRE_EVERY_TICKS: u32 = 4; // 15 shots/s

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
    pub fn lerp(a: V2, b: V2, t: f32) -> V2 {
        V2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    }
}

/// Anything that moves keeps its previous-tick position for render interpolation.
#[derive(Clone, Copy)]
pub struct Mover {
    pub prev: V2,
    pub pos: V2,
    pub vel: V2,
}

#[derive(Default, Clone, Copy)]
pub struct SimInput {
    /// Magnitude <= 1.
    pub move_dir: V2,
    /// Some(unit-ish direction) while the aim stick is held past its deadzone.
    pub aim_dir: Option<V2>,
}

pub struct Sim {
    pub bounds: V2,
    pub player: Mover,
    pub bullets: Vec<Mover>,
    pub stress: Vec<Mover>,
    fire_cooldown: u32,
    rng: u32,
}

impl Sim {
    pub fn new(bounds: V2) -> Self {
        let c = V2::new(bounds.x * 0.5, bounds.y * 0.5);
        Self {
            bounds,
            player: Mover { prev: c, pos: c, vel: V2::default() },
            bullets: Vec::new(),
            stress: Vec::new(),
            fire_cooldown: 0,
            rng: 0x1234_5678,
        }
    }

    fn rand(&mut self) -> f32 {
        // xorshift32
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
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

    pub fn step(&mut self, input: SimInput) {
        let b = self.bounds;
        let half = PLAYER_SIZE * 0.5;

        let p = &mut self.player;
        p.prev = p.pos;
        p.pos.x = (p.pos.x + input.move_dir.x * PLAYER_SPEED * DT).clamp(half, b.x - half);
        p.pos.y = (p.pos.y + input.move_dir.y * PLAYER_SPEED * DT).clamp(half, b.y - half);
        let origin = p.pos;

        for m in &mut self.bullets {
            m.prev = m.pos;
            m.pos.x += m.vel.x * DT;
            m.pos.y += m.vel.y * DT;
        }
        self.bullets
            .retain(|m| m.pos.x > -20.0 && m.pos.y > -20.0 && m.pos.x < b.x + 20.0 && m.pos.y < b.y + 20.0);

        self.fire_cooldown = self.fire_cooldown.saturating_sub(1);
        if let Some(d) = input.aim_dir {
            if self.fire_cooldown == 0 {
                let l = d.len().max(1e-6);
                let vel = V2::new(d.x / l * BULLET_SPEED, d.y / l * BULLET_SPEED);
                self.bullets.push(Mover { prev: origin, pos: origin, vel });
                self.fire_cooldown = FIRE_EVERY_TICKS;
            }
        }

        for m in &mut self.stress {
            m.prev = m.pos;
            m.pos.x += m.vel.x * DT;
            m.pos.y += m.vel.y * DT;
            if m.pos.x < 0.0 || m.pos.x > b.x {
                m.vel.x = -m.vel.x;
                m.pos.x = m.pos.x.clamp(0.0, b.x);
            }
            if m.pos.y < 0.0 || m.pos.y > b.y {
                m.vel.y = -m.vel.y;
                m.pos.y = m.pos.y.clamp(0.0, b.y);
            }
        }
    }
}
