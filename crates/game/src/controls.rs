//! Touch -> quantized [`PlayerInput`] for the four control schemes (issue #7), ported from
//! the feel spike, and the three fire modes (issue #15). Schemes and fire modes only
//! change how touches become input; aim assist, auto-aim and the gun's fire cap are sim
//! rules.

use crate::TouchPhase;
use render::{ButtonView, Overlay, StickView};
use sim::{Buttons, MOVE_BUCKETS, PlayerInput};
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

/// Stick travel in points.
const STICK_RADIUS: f32 = 60.0;
/// Scheme B: a touch this close to a fixed base grabs it.
const FIXED_GRAB_RADIUS: f32 = 130.0;
/// Fixed stick bases sit this far in from the safe-area corner, in points.
const BASE_INSET: f32 = 100.0;
const DODGE_RADIUS: f32 = 34.0;
/// Touches register a little outside the drawn button.
const DODGE_HIT_RADIUS: f32 = DODGE_RADIUS * 1.3;
/// Dodge button offset from the right stick base: up and toward the edge, clear of the
/// stick's travel.
const DODGE_OFFSET: [f32; 2] = [56.0, -110.0];
const VENT_RADIUS: f32 = 26.0;
const VENT_HIT_RADIUS: f32 = VENT_RADIUS * 1.3;
/// Vent button offset from the right stick base: up and inward, clear of the dodge
/// button and the stick's travel.
const VENT_OFFSET: [f32; 2] = [-50.0, -130.0];
const MOVE_DEADZONE: f32 = 0.1;
const AIM_DEADZONE: f32 = 0.2;
/// Scheme C: a right-half swipe this far within [`FLICK_TIME`] rolls in its direction.
const FLICK_DISTANCE: f32 = 45.0;
const FLICK_TIME: Duration = Duration::from_millis(200);
/// A touch that starts on the dodge button and slides this far rolls that way at once;
/// lifted sooner, it's a tap: a roll the way the player moves.
const DODGE_SWIPE: f32 = 20.0;

/// Touch control schemes, chosen in the Controls setting.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scheme {
    /// A: floating twin sticks (left half moves, right half aims and fires); dodge button.
    FloatingSticks,
    /// B: stick bases anchored near the bottom corners; dodge button.
    FixedSticks,
    /// C: floating move stick; holding the right half fires at the nearest target and a
    /// flick there dodges.
    AutoAim,
    /// D: like A, with aim bent toward targets by the assist strength.
    AimAssist,
    /// E: C with B's anchored move stick.
    FixedAutoAim,
    /// F: Brawl Stars / Soul Knight style. B's anchored sticks, but the right one is a
    /// fire button: held still it fires at the nearest target, dragged it aims by hand.
    /// Dodge button.
    #[default]
    FireButton,
}

impl Scheme {
    /// The right half fires at the nearest target and flicks dodge (C, E).
    const fn auto_aim(self) -> bool {
        matches!(self, Self::AutoAim | Self::FixedAutoAim)
    }

    /// Whether `side`'s stick (0 move, 1 aim) is anchored at its base.
    const fn fixed(self, side: usize) -> bool {
        match self {
            Self::FixedSticks | Self::FireButton => true,
            Self::FixedAutoAim => side == 0,
            Self::FloatingSticks | Self::AutoAim | Self::AimAssist => false,
        }
    }
}

/// How the aim side's touches fire, chosen in settings.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FireMode {
    /// Auto: fires (at the gun's cap) while the aim stick is held past its deadzone.
    #[default]
    Hold,
    /// Semi-auto: each touch-down on the aim side fires one shot toward the touch.
    /// Dragging aims without firing.
    Tap,
    /// Drag to aim; lifting the finger fires one shot along the aim.
    Release,
    /// Each press of the phone's volume-up button fires one shot (Swift reports them; a
    /// held button repeats). The aim side only aims; with it released, shots go along the
    /// last aim. Auto-aim schemes and an undeflected fire button fire at the nearest target.
    Trigger,
}

/// View size and safe-area insets, in points.
#[derive(uniffi::Record, Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub point_width: f32,
    pub point_height: f32,
    pub safe_top: f32,
    pub safe_left: f32,
    pub safe_bottom: f32,
    pub safe_right: f32,
}

/// Where the on-screen controls sit for a viewport.
struct Layout {
    width: f32,
    bases: [[f32; 2]; 2],
    dodge: [f32; 2],
    vent: [f32; 2],
}

impl Layout {
    fn new(v: &Viewport) -> Self {
        let y = v.point_height - v.safe_bottom - BASE_INSET;
        let right = [v.point_width - v.safe_right - BASE_INSET, y];
        Self {
            width: v.point_width,
            bases: [[v.safe_left + BASE_INSET, y], right],
            dodge: [right[0] + DODGE_OFFSET[0], right[1] + DODGE_OFFSET[1]],
            vent: [right[0] + VENT_OFFSET[0], right[1] + VENT_OFFSET[1]],
        }
    }
}

struct Stick {
    id: u64,
    origin: [f32; 2],
    cur: [f32; 2],
    /// Fixed sticks keep their base; floating ones drag it along past the radius.
    fixed: bool,
    start: [f32; 2],
    started: Instant,
    flicked: bool,
}

impl Stick {
    fn new(id: u64, origin: [f32; 2], p: [f32; 2], fixed: bool) -> Self {
        Self {
            id,
            origin,
            cur: p,
            fixed,
            start: p,
            started: Instant::now(),
            flicked: false,
        }
    }

    fn drag(&mut self, p: [f32; 2]) {
        self.cur = p;
        let [dx, dy] = sub(p, self.origin);
        let len = dx.hypot(dy);
        if !self.fixed && len > STICK_RADIUS {
            let k = (len - STICK_RADIUS) / len;
            self.origin = [dx.mul_add(k, self.origin[0]), dy.mul_add(k, self.origin[1])];
        }
    }

    /// Deflection as (angle in turns `0..1` from +x toward +y, magnitude `0..=1`).
    fn polar(&self) -> (f32, f32) {
        let [dx, dy] = sub(self.cur, self.origin);
        (turns(dx, dy), (dx.hypot(dy) / STICK_RADIUS).min(1.0))
    }

    fn knob(&self) -> [f32; 2] {
        let [dx, dy] = sub(self.cur, self.origin);
        let k = STICK_RADIUS / dx.hypot(dy).max(STICK_RADIUS);
        [dx.mul_add(k, self.origin[0]), dy.mul_add(k, self.origin[1])]
    }
}

/// One shot waiting for the next sim tick (tap and release fire modes). The sim's fire
/// cap drops it if the gun isn't ready.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shot {
    /// Along this angle, in turns `0..1` from +x toward +y.
    At(f32),
    /// Scheme C: at the nearest target.
    Auto,
}

/// A dodge waiting for the next sim tick.
#[derive(Clone, Copy)]
enum Dodge {
    Button,
    /// Roll along a flick or a swipe off the dodge button, whatever the move stick says.
    Flick {
        move_dir: u8,
    },
}

pub struct Controls {
    scheme: Scheme,
    fire_mode: FireMode,
    /// Aim assist for scheme D, `0..=1`.
    assist: f32,
    layout: Layout,
    sticks: [Option<Stick>; 2],
    dodge: Option<Dodge>,
    /// A touch that started on the dodge button and hasn't swiped yet: (id, start).
    dodge_touch: Option<(u64, [f32; 2])>,
    shot: Option<Shot>,
    /// Where slot 0 was last drawn, in view points; tap-to-fire aims from here.
    player_view: Option<[f32; 2]>,
    /// A vent tap waiting for the next sim tick.
    vent: bool,
    /// The last aim stick angle, in turns: where a trigger shot goes with no stick held.
    last_aim: f32,
    /// A restart tap waiting for the next sim tick.
    restart: bool,
}

impl Controls {
    pub fn new(viewport: &Viewport) -> Self {
        Self {
            scheme: Scheme::default(),
            fire_mode: FireMode::default(),
            assist: 0.5,
            layout: Layout::new(viewport),
            sticks: [None, None],
            dodge: None,
            dodge_touch: None,
            shot: None,
            player_view: None,
            vent: false,
            last_aim: 0.0,
            restart: false,
        }
    }

    pub const fn request_restart(&mut self) {
        self.restart = true;
    }

    /// Stick origins belong to the old layout; rotation cancels the touches anyway.
    pub fn set_viewport(&mut self, viewport: &Viewport) {
        self.layout = Layout::new(viewport);
        self.sticks = [None, None];
    }

    /// Drops held sticks and a pending dodge, shot or vent (pause); a pending restart
    /// survives.
    pub const fn release(&mut self) {
        self.sticks = [None, None];
        self.dodge = None;
        self.dodge_touch = None;
        self.shot = None;
        self.vent = false;
    }

    pub const fn set_scheme(&mut self, scheme: Scheme) {
        self.scheme = scheme;
        self.sticks = [None, None];
    }

    pub const fn set_fire_mode(&mut self, mode: FireMode) {
        self.fire_mode = mode;
        self.sticks = [None, None];
        self.shot = None;
    }

    /// One [`FireMode::Trigger`] shot, on the next tick.
    pub fn pull_trigger(&mut self) {
        let aim = self.sticks[1]
            .as_ref()
            .map(Stick::polar)
            .filter(|&(_, mag)| mag > AIM_DEADZONE);
        self.shot = Some(match (self.scheme, aim) {
            (s, _) if s.auto_aim() => Shot::Auto,
            (_, Some((t, _))) => Shot::At(t),
            (Scheme::FireButton, None) => Shot::Auto,
            (_, None) => Shot::At(self.last_aim),
        });
    }

    /// A dodge from outside the touch layout (the volume-down button).
    pub const fn press_dodge(&mut self) {
        self.dodge = Some(Dodge::Button);
    }

    pub const fn set_player_view(&mut self, at: Option<[f32; 2]>) {
        self.player_view = at;
    }

    pub const fn set_assist(&mut self, strength: f32) {
        self.assist = strength.clamp(0.0, 1.0);
    }

    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        let p = [x, y];
        match phase {
            TouchPhase::Began => self.begin(id, p),
            TouchPhase::Moved => {
                if let Some((touch, start)) = self.dodge_touch
                    && touch == id
                    && dist(p, start) > DODGE_SWIPE
                {
                    let [dx, dy] = sub(p, start);
                    self.dodge = Some(Dodge::Flick {
                        move_dir: move_bucket(turns(dx, dy)),
                    });
                    self.dodge_touch = None;
                }
                for stick in self.sticks.iter_mut().flatten() {
                    if stick.id == id {
                        stick.drag(p);
                    }
                }
                if self.scheme.auto_aim()
                    && let [_, Some(right)] = &mut self.sticks
                    && right.id == id
                    && !right.flicked
                {
                    let [dx, dy] = sub(p, right.start);
                    if dx.hypot(dy) > FLICK_DISTANCE && right.started.elapsed() < FLICK_TIME {
                        right.flicked = true;
                        self.dodge = Some(Dodge::Flick {
                            move_dir: move_bucket(turns(dx, dy)),
                        });
                    }
                }
            }
            TouchPhase::Ended => {
                if self.dodge_touch.is_some_and(|(touch, _)| touch == id) {
                    self.dodge = Some(Dodge::Button);
                    self.dodge_touch = None;
                }
                if self.fire_mode == FireMode::Release
                    && let [_, Some(right)] = &self.sticks
                    && right.id == id
                {
                    self.shot = self.aimed_shot(right);
                }
                for slot in &mut self.sticks {
                    if slot.as_ref().is_some_and(|s| s.id == id) {
                        *slot = None;
                    }
                }
            }
        }
    }

    fn begin(&mut self, id: u64, p: [f32; 2]) {
        if dist(p, self.layout.vent) < VENT_HIT_RADIUS {
            self.vent = true;
            return;
        }
        if !self.scheme.auto_aim() && dist(p, self.layout.dodge) < DODGE_HIT_RADIUS {
            self.dodge_touch = Some((id, p));
            return;
        }
        let side = usize::from(p[0] >= self.layout.width * 0.5);
        let (Some(slot), Some(&base)) = (self.sticks.get_mut(side), self.layout.bases.get(side))
        else {
            return;
        };
        if slot.is_some() {
            return;
        }
        if self.scheme.fixed(side) {
            if dist(p, base) < FIXED_GRAB_RADIUS {
                // Scheme F's fire button measures a drag from where the thumb landed, so
                // an off-center press still auto-aims.
                let origin = if self.scheme == Scheme::FireButton && side == 1 {
                    p
                } else {
                    base
                };
                let mut stick = Stick::new(id, origin, p, true);
                stick.drag(p);
                *slot = Some(stick);
            }
        } else {
            *slot = Some(Stick::new(id, p, p, false));
        }
        if side == 1
            && self.fire_mode == FireMode::Tap
            && let [_, Some(right)] = &self.sticks
        {
            // A fixed stick's deflection already points at the touch; a floating one
            // starts undeflected, so aim from the player on screen to the touch.
            let toward_touch = self.player_view.map(|from| {
                let [dx, dy] = sub(p, from);
                Shot::At(turns(dx, dy))
            });
            self.shot = self.aimed_shot(right).or(toward_touch);
        }
    }

    /// A single shot along `stick`'s aim: auto-aimed in scheme C, else only past the aim
    /// deadzone.
    fn aimed_shot(&self, stick: &Stick) -> Option<Shot> {
        if self.scheme.auto_aim() {
            return Some(Shot::Auto);
        }
        let (t, mag) = stick.polar();
        if mag > AIM_DEADZONE {
            Some(Shot::At(t))
        } else {
            (self.scheme == Scheme::FireButton).then_some(Shot::Auto)
        }
    }

    /// How far the aim is pushed, `0..=1`, for the camera's aim look: the aim stick's
    /// deflection past its deadzone, or full while scheme C's fire side is held.
    pub fn aim_push(&self) -> f32 {
        let [_, right] = &self.sticks;
        match (self.scheme, right) {
            (_, None) => 0.0,
            (Scheme::AutoAim | Scheme::FixedAutoAim, Some(_)) => 1.0,
            (_, Some(stick)) => {
                let (_, mag) = stick.polar();
                ((mag - AIM_DEADZONE) / (1.0 - AIM_DEADZONE)).clamp(0.0, 1.0)
            }
        }
    }

    /// Input for the next sim tick. A pending dodge, vent or restart goes out once, on the
    /// first tick that actually runs.
    pub fn next_input(&mut self) -> PlayerInput {
        let mut input = PlayerInput::default();
        if std::mem::take(&mut self.restart) {
            input.buttons |= Buttons::RESTART;
        }
        if std::mem::take(&mut self.vent) {
            input.buttons |= Buttons::VENT;
        }
        let [left, right] = &self.sticks;
        if let Some((t, mag)) = left.as_ref().map(Stick::polar)
            && mag > MOVE_DEADZONE
        {
            input.move_dir = move_bucket(t);
            input.move_mag = u8::try_from(quantize(mag, 255)).unwrap_or(u8::MAX);
        }
        let aim = right
            .as_ref()
            .map(Stick::polar)
            .filter(|&(_, mag)| mag > AIM_DEADZONE);
        // A deflected aim stick turns the player even when it doesn't fire (tap and
        // release); a shot below overrides the aim with its own.
        if let Some((t, _)) = aim
            && !self.scheme.auto_aim()
        {
            input.aim = aim_angle(t);
            input.buttons |= Buttons::AIM;
            self.last_aim = t;
        }
        // Hold fires from the held stick; tap and release only through pending shots.
        let held = self.fire_mode == FireMode::Hold;
        let shot = match self.scheme {
            _ if self.fire_mode == FireMode::Trigger => None,
            Scheme::FireButton if held => right
                .as_ref()
                .map(|_| aim.map_or(Shot::Auto, |(t, _)| Shot::At(t))),
            Scheme::FireButton => None,
            Scheme::FloatingSticks | Scheme::FixedSticks | Scheme::AimAssist => {
                aim.filter(|_| held).map(|(t, _)| Shot::At(t))
            }
            Scheme::AutoAim | Scheme::FixedAutoAim => {
                (held && right.is_some()).then_some(Shot::Auto)
            }
        };
        match self.shot.take().or(shot) {
            Some(Shot::At(t)) => {
                input.aim = aim_angle(t);
                input.buttons |= Buttons::FIRE;
            }
            Some(Shot::Auto) => input.buttons |= Buttons::FIRE | Buttons::AUTO_AIM,
            None => {}
        }
        if self.scheme == Scheme::AimAssist {
            input.assist = u8::try_from(quantize(self.assist, 255)).unwrap_or(u8::MAX);
        }
        match self.dodge.take() {
            Some(Dodge::Button) => input.buttons |= Buttons::DODGE,
            Some(Dodge::Flick { move_dir }) => {
                input.buttons |= Buttons::DODGE;
                input.move_dir = move_dir;
                input.move_mag = u8::MAX;
            }
            None => {}
        }
        input
    }

    pub fn overlay(&self, roll_ready: bool, vent_ready: bool) -> Overlay {
        let mut overlay = Overlay::default();
        for (side, ((view, stick), &base)) in overlay
            .sticks
            .iter_mut()
            .zip(&self.sticks)
            .zip(&self.layout.bases)
            .enumerate()
        {
            *view = match stick {
                Some(s) => Some(StickView {
                    base: s.origin,
                    radius: STICK_RADIUS,
                    knob: s.knob(),
                    active: true,
                }),
                None if self.scheme.fixed(side) => Some(StickView {
                    base,
                    radius: STICK_RADIUS,
                    knob: base,
                    active: false,
                }),
                None => None,
            };
        }
        overlay.dodge = (!self.scheme.auto_aim()).then_some(ButtonView {
            center: self.layout.dodge,
            radius: DODGE_RADIUS,
            ready: roll_ready,
        });
        overlay.vent = Some(ButtonView {
            center: self.layout.vent,
            radius: VENT_RADIUS,
            ready: vent_ready,
        });
        overlay
    }
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let [dx, dy] = sub(a, b);
    dx.hypot(dy)
}

/// Angle of (`dx`, `dy`) in turns `0..1` from +x toward +y.
fn turns(dx: f32, dy: f32) -> f32 {
    (dy.atan2(dx) / TAU).rem_euclid(1.0)
}

/// `turns` as a sim angle: one full turn = 65536, wrapping.
fn aim_angle(turns: f32) -> u16 {
    u16::try_from(quantize(turns, 1 << 16) & 0xFFFF).unwrap_or(0)
}

fn move_bucket(turns: f32) -> u8 {
    let buckets = u32::from(MOVE_BUCKETS);
    // Rounding up to `buckets` wraps to bucket 0.
    quantize(turns, buckets)
        .checked_rem(buckets)
        .and_then(|b| u8::try_from(b).ok())
        .unwrap_or(0)
}

/// Rounds `x` in `0..=1` to the nearest step in `0..=max`.
// Float -> int has no TryFrom; `x` is clamped so the result is always in range.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn quantize(x: f32, max: u32) -> u32 {
    (x.clamp(0.0, 1.0) * max as f32).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls(scheme: Scheme) -> Controls {
        let mut c = Controls::new(&Viewport {
            point_width: 852.0,
            point_height: 393.0,
            safe_left: 59.0,
            safe_right: 59.0,
            safe_bottom: 21.0,
            ..Viewport::default()
        });
        c.set_scheme(scheme);
        c
    }

    #[test]
    fn a_dodge_button_tap_sends_one_dodge_on_release() {
        let mut c = controls(Scheme::FixedSticks);
        let [x, y] = c.layout.dodge;
        c.touch(1, TouchPhase::Began, x, y);
        assert!(!c.next_input().buttons.contains(Buttons::DODGE));
        c.touch(1, TouchPhase::Ended, x, y);
        assert!(c.next_input().buttons.contains(Buttons::DODGE));
        assert!(!c.next_input().buttons.contains(Buttons::DODGE));
    }

    #[test]
    fn a_swipe_off_the_dodge_button_rolls_that_way_at_once() {
        let mut c = controls(Scheme::FireButton);
        let [x, y] = c.layout.dodge;
        c.touch(1, TouchPhase::Began, x, y);
        c.touch(1, TouchPhase::Moved, x, y - 30.0); // straight up
        let input = c.next_input();
        assert!(input.buttons.contains(Buttons::DODGE));
        assert_eq!((input.move_dir, input.move_mag), (24, 255));
        c.touch(1, TouchPhase::Ended, x, y - 30.0);
        assert!(!c.next_input().buttons.contains(Buttons::DODGE), "one roll");
    }

    #[test]
    fn vent_button_sends_one_vent_in_every_scheme() {
        for scheme in [Scheme::FixedSticks, Scheme::AutoAim] {
            let mut c = controls(scheme);
            let [x, y] = c.layout.vent;
            c.touch(1, TouchPhase::Began, x, y);
            let input = c.next_input();
            assert!(input.buttons.contains(Buttons::VENT), "{scheme:?}");
            assert!(
                !input.buttons.contains(Buttons::FIRE),
                "{scheme:?}: not a stick"
            );
            assert!(!c.next_input().buttons.contains(Buttons::VENT));
        }
    }

    #[test]
    fn restart_tap_sends_one_restart() {
        let mut c = controls(Scheme::FixedSticks);
        c.request_restart();
        assert!(c.next_input().buttons.contains(Buttons::RESTART));
        assert!(!c.next_input().buttons.contains(Buttons::RESTART));
    }

    #[test]
    fn fixed_sticks_only_grab_near_their_base() {
        let mut c = controls(Scheme::FixedSticks);
        let [x, y] = c.layout.bases[0];
        c.touch(1, TouchPhase::Began, x, 20.0); // far above the left base
        c.touch(2, TouchPhase::Began, x + STICK_RADIUS, y); // full right
        let input = c.next_input();
        assert_eq!((input.move_dir, input.move_mag), (0, 255));
    }

    fn fired(input: PlayerInput) -> Option<u16> {
        input.buttons.contains(Buttons::FIRE).then_some(input.aim)
    }

    /// (fired angle, aim-only angle) of one input.
    fn aimed(input: PlayerInput) -> (Option<u16>, Option<u16>) {
        let only_aim = input.buttons.contains(Buttons::AIM) && fired(input).is_none();
        (fired(input), only_aim.then_some(input.aim))
    }

    #[test]
    fn hold_fires_every_tick_the_aim_stick_is_deflected() {
        let mut c = controls(Scheme::FixedSticks);
        let [x, y] = c.layout.bases[1];
        c.touch(1, TouchPhase::Began, x, y + STICK_RADIUS); // straight down
        assert_eq!(fired(c.next_input()), Some(16384));
        assert_eq!(fired(c.next_input()), Some(16384));
        c.touch(1, TouchPhase::Ended, x, y + STICK_RADIUS);
        assert_eq!(fired(c.next_input()), None);
    }

    #[test]
    fn tap_fires_once_toward_the_touch_and_dragging_only_aims() {
        // Fixed stick: the touch's offset from the base is the aim.
        let mut c = controls(Scheme::FixedSticks);
        c.set_fire_mode(FireMode::Tap);
        let [x, y] = c.layout.bases[1];
        c.touch(1, TouchPhase::Began, x - STICK_RADIUS, y); // straight left
        assert_eq!(fired(c.next_input()), Some(32768));
        c.touch(1, TouchPhase::Moved, x, y + STICK_RADIUS);
        assert_eq!(
            aimed(c.next_input()),
            (None, Some(16384)),
            "held and dragged: aims down, no more shots"
        );
        c.touch(1, TouchPhase::Ended, x, y + STICK_RADIUS);
        c.touch(2, TouchPhase::Began, x, y - STICK_RADIUS); // straight up
        assert_eq!(fired(c.next_input()), Some(49152), "each touch-down fires");

        // Floating stick: aim from the player on screen to the touch.
        let mut c = controls(Scheme::FloatingSticks);
        c.set_fire_mode(FireMode::Tap);
        c.set_player_view(Some([400.0, 200.0]));
        c.touch(1, TouchPhase::Began, 600.0, 200.0);
        assert_eq!(fired(c.next_input()), Some(0));
        assert_eq!(fired(c.next_input()), None);
    }

    #[test]
    fn release_fires_once_along_the_dragged_aim_when_lifted() {
        let mut c = controls(Scheme::FloatingSticks);
        c.set_fire_mode(FireMode::Release);
        c.touch(1, TouchPhase::Began, 600.0, 200.0);
        c.touch(1, TouchPhase::Moved, 600.0, 200.0 + STICK_RADIUS); // aim down
        assert_eq!(
            aimed(c.next_input()),
            (None, Some(16384)),
            "dragging only aims"
        );
        c.touch(1, TouchPhase::Ended, 600.0, 200.0 + STICK_RADIUS);
        assert_eq!(fired(c.next_input()), Some(16384));
        assert_eq!(fired(c.next_input()), None);
        // A release inside the deadzone is a cancel.
        c.touch(2, TouchPhase::Began, 600.0, 200.0);
        c.touch(2, TouchPhase::Ended, 600.0, 200.0);
        assert_eq!(fired(c.next_input()), None);
    }

    #[test]
    fn scheme_c_taps_and_releases_fire_one_auto_aimed_shot() {
        for mode in [FireMode::Tap, FireMode::Release] {
            let mut c = controls(Scheme::AutoAim);
            c.set_fire_mode(mode);
            c.touch(1, TouchPhase::Began, 600.0, 200.0);
            let first = c.next_input();
            c.touch(1, TouchPhase::Ended, 600.0, 200.0);
            let second = c.next_input();
            let shots = [first, second]
                .iter()
                .filter(|i| i.buttons.contains(Buttons::FIRE | Buttons::AUTO_AIM))
                .count();
            assert_eq!(shots, 1, "{mode:?}");
        }
    }

    #[test]
    fn flick_dodges_along_the_flick() {
        for scheme in [Scheme::AutoAim, Scheme::FixedAutoAim] {
            let mut c = controls(scheme);
            c.touch(1, TouchPhase::Began, 600.0, 200.0);
            c.touch(1, TouchPhase::Moved, 600.0, 150.0); // 50 pt straight up
            let input = c.next_input();
            assert!(
                input
                    .buttons
                    .contains(Buttons::DODGE | Buttons::FIRE | Buttons::AUTO_AIM),
                "{scheme:?}"
            );
            assert_eq!((input.move_dir, input.move_mag), (24, 255), "{scheme:?}");
        }
    }

    #[test]
    fn trigger_mode_aims_with_the_stick_and_fires_only_on_the_trigger() {
        let mut c = controls(Scheme::FixedSticks);
        c.set_fire_mode(FireMode::Trigger);
        let [x, y] = c.layout.bases[1];
        c.touch(1, TouchPhase::Began, x, y + STICK_RADIUS); // straight down
        let (fired_angle, aimed_angle) = aimed(c.next_input());
        assert_eq!((fired_angle, aimed_angle), (None, Some(1 << 14)));
        c.pull_trigger();
        assert_eq!(fired(c.next_input()), Some(1 << 14));
        assert_eq!(fired(c.next_input()), None, "one shot per pull");
        c.touch(1, TouchPhase::Ended, x, y + STICK_RADIUS);
        c.pull_trigger();
        assert_eq!(fired(c.next_input()), Some(1 << 14), "along the last aim");
    }

    #[test]
    fn scheme_f_fires_at_the_nearest_target_until_dragged_to_aim() {
        let mut c = controls(Scheme::FireButton);
        let [x, y] = c.layout.bases[1];
        c.touch(1, TouchPhase::Began, x, y);
        let input = c.next_input();
        assert!(input.buttons.contains(Buttons::FIRE | Buttons::AUTO_AIM));
        c.touch(1, TouchPhase::Moved, x, y + STICK_RADIUS); // dragged straight down
        let input = c.next_input();
        assert!(!input.buttons.contains(Buttons::AUTO_AIM));
        assert_eq!(fired(input), Some(1 << 14));
        c.touch(1, TouchPhase::Ended, x, y + STICK_RADIUS);
        assert_eq!(fired(c.next_input()), None);
    }

    #[test]
    fn scheme_e_anchors_only_the_move_stick() {
        let mut c = controls(Scheme::FixedAutoAim);
        let [x, y] = c.layout.bases[0];
        c.touch(1, TouchPhase::Began, x, 20.0); // far above the left base: ignored
        c.touch(2, TouchPhase::Began, x + STICK_RADIUS, y); // full right from the base
        let input = c.next_input();
        assert_eq!((input.move_dir, input.move_mag), (0, 255));
        let overlay = c.overlay(true, true);
        assert!(overlay.sticks[1].is_none(), "the fire side floats");
    }
}
