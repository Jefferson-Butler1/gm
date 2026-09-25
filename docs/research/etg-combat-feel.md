# What are Enter the Gungeon's actual combat numbers?

Research ticket. Question: what are ETG's real movement, dodge roll, gun, health, enemy and camera numbers? We want to tune our iPhone twin-stick roguelite closer to ETG.

Researched 2026-09-24.

**Sources.** Numbers come from two places:
- **`src`**: the game's own data, taken from a community decompile / asset export of ETG in [fedes1to/EtG-source](https://github.com/fedes1to/EtG-source). This means the C# scripts plus the serialized prefab values for the player, enemies and rooms. The snapshot was pushed 2022-10. It is the highest-trust source, because these are the values the game ships with.
- **`wiki`**: the [official ETG wiki (wiki.gg)](https://enterthegungeon.wiki.gg). Gun stat infoboxes come from here. Its values are derived from game data, but I did not cross-check them against gun prefabs.

**Units.** 1 Unity unit = 1 tile = 16 px. The renderer snaps positions to `x * 16 / 16` ([Pixelator.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Pixelator.cs), e.g. lines 1325, 1867). The camera renders a 480×270 "macro pixel" buffer. So all speeds below are in tiles/s.

Links to `github.com/fedes1to/EtG-source` point at `ExportedProject/Assets/...` in that repo.

## TL;DR

- **Movement is slow and instant.** Speed is 7 tiles/s, with no acceleration: velocity = input direction × 7 every frame. Ours is 420 pt/s = 13.1 tiles/s, about **1.9× ETG**.
- **The roll is long and only partly invulnerable. It is not a fast dash.**
  - It moves 5 tiles in 0.65 s: average 7.7 tiles/s, only about 1.1× walk speed. It is front-loaded, at roughly 10.7 tiles/s early and 4.3 tiles/s at the end.
  - **I-frames cover the first 5/9 of the roll (about 0.36 s, about 3.7 tiles).** The last about 0.29 s is vulnerable.
  - There is no cooldown timer, but you cannot shoot while rolling. Roll cadence is therefore capped at about 1 per 0.65 s.
  - Roll distance / move speed: ETG 5 / 7 = **0.71 s of walking**. Ours 170 / 420 = 0.40 s.
- **Reloads are the core rhythm.** Starter pistols have 5–10 round clips and reload in **0.9–1.2 s**.
  - The Marine Sidearm is 4 shots/s, 10 rounds, 1.2 s reload. That is about 2.9 shots/s sustained, versus our 7.5 shots/s with no reload.
  - Common non-starters reload in 0.5–1.0 s.
- **Enemy bullets are slow and roughly equal to player speed.**
  - Bullet Kin bullet 8 tiles/s → **bullet / player = 1.14**.
  - Shotgun Kin pellets 5 tiles/s → **0.71**.
  - Ours: 300 / 420 = 0.71. **If we slow the player to ETG speed without slowing enemy bullets, our ratio jumps to 1.34**, faster than anything in ETG's basic enemies.
- **Player bullet / enemy bullet** is about 3.1 in ETG (25 vs 8). Ours is 3.0. That matches already.
- **Health:** 3 hearts = **6 half-heart hits**. The Marine also has +1 armor, so 7 hits. On hit you get **1.0 s invulnerability plus 1.4 s where bullets pass through you**. Ours: 5 hits, 0.75 s.
- **Bullet Kin fires every 1.6 s.** That is identical to our shooter.
- **Screen:** 30 × 16.9 tiles, landscape. A median Keep combat room is 24 × 20 tiles.

## Numbers

### Player

| Thing | ETG value | Tiles/s etc. | Source |
|---|---|---|---|
| Move speed | `MovementSpeed` base stat = 7 | 7 tiles/s | [src PlayerStatsShared.prefab](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/Asset_Bundles/brave_resources_001/resourcesbundle/PlayerStatsShared.prefab) `BaseStatValues[0]`; [wiki Movement Speed](https://enterthegungeon.wiki.gg/wiki/Movement_Speed) |
| Acceleration | None (velocity set directly each frame; ice floors are the exception) | instant | [src PlayerController.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/PlayerController.cs) ~l.1924 `voluntaryVel = m_playerCommandedDirection * stats.MovementSpeed` |
| Out-of-combat speed | ×1.5 (option `IncreaseSpeedOutOfCombat`) | 10.5 | same, ~l.1900–1923 |
| Roll time | `rollStats.time: 0.65` | 0.65 s | [src PlayerMarine.prefab](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/Asset_Bundles/brave_resources_001/resourcesbundle/PlayerMarine.prefab) l.98. The wiki says "about 0.7 seconds" ([wiki Dodge Roll (Move)](https://enterthegungeon.wiki.gg/wiki/Dodge_Roll_(Move))) |
| Roll distance | `rollStats.distance: 5` | 5 tiles | PlayerMarine.prefab l.99 |
| Roll speed profile | AnimationCurve (normalized distance vs time). Slope ≈1.39 until t≈0.47, then ≈0.56 at the end | ≈10.7 tiles/s early → ≈4.3 tiles/s late (my evaluation of the curve) | PlayerMarine.prefab l.100–126; `GetDodgeRollSpeed()` in PlayerController.cs ~l.7930 |
| Roll i-frames | Dodge anim: 9 frames played over the roll time; `invulnerableFrame` = frames 0–4 | first 5/9 ≈ **0.36 s**, ≈3.7 tiles | [src MarineAnimation.prefab](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/Asset_Bundles/shared_auto_001/sprites/characters/animations/MarineAnimation.prefab) (all `dodge*` clips: inv `111110000`, grounded `000011111`); fps override = frames / roll time (PlayerController.cs `PlayDodgeRollAnimation`); `QueryGroundedFrame` hard-codes `5f/9f * rollTime`. The wiki says "first half" |
| Roll cooldown | None. A new roll is allowed once the current one ends (depth limit 1; 2 with Pegasus Boots) | 0 s | PlayerController.cs `CheckDodgeRollDepth` / `StartDodgeRoll` ~l.7187–7242 |
| Roll requires a direction | Yes. `StartDodgeRoll` returns false if the direction is zero | — | PlayerController.cs l.7214 |
| Roll direction | Locked at start. 8-way on keyboard, 360° on gamepad | — | `lockedDodgeRollDirection`; [wiki](https://enterthegungeon.wiki.gg/wiki/Dodge_Roll_(Move)) |
| Shoot during roll | **No** (`m_CanAttack` = `!IsDodgeRolling \|\| IsSlidingOverSurface`, i.e. only while sliding over a table) | — | PlayerController.cs l.876–881 |
| Reload during roll | Reload keeps ticking. The reload coroutine has no roll check | — | [src Gun.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Gun.cs) `HandleReload` ~l.5204 (code reading, **not tested in game**) |
| Pits during roll | Can't fall until the roll is fully grounded | — | PlayerController.cs l.5291 (`m_dodgeRollState != OnGround` → no fall); [wiki](https://enterthegungeon.wiki.gg/wiki/Dodge_Roll_(Move)) |
| Roll contact damage | 3 (`rollDamage`). Kills enemies with <3 HP and passes through them | — | PlayerStatsShared.prefab `rollDamage: 3`; PlayerController.cs ~l.3970 |
| Roll distance scaling | `distance × (1 + 0.5·(moveSpeedMult − 1))`. Speed items lengthen the roll by half as much | — | [src DodgeRollStats.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/DodgeRollStats.cs) |
| Health | `maximumHealth: 3` hearts, `quantizedIncrement: 0.5` | 6 half-heart hits | PlayerMarine.prefab l.303–315 (HealthHaver) |
| Starting armor (Marine) | `currentArmor: 1`. One armor absorbs one full hit, and losing it fires a free blank | +1 hit | PlayerMarine.prefab l.313; [src HealthHaver.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/HealthHaver.cs) l.737–744; `OnLostArmor → ForceBlank()` in PlayerController.cs ~l.4418 |
| Post-hit invulnerability | `invulnerabilityPeriod: 1` | 1.0 s | PlayerMarine.prefab l.315; HealthHaver.cs `HandleInvulnerablePeriod` |
| Post-hit incorporeality | `incorporealityTime: 1.4`. Ignores enemy bodies, hitboxes and projectiles; blinks every 0.12 s | 1.4 s | PlayerMarine.prefab l.307–308; HealthHaver.cs `IncorporealityOnHit` l.1283 |
| Enemy bullet damage | 0.5 (half heart) | — | BulletMan.prefab `ProjectileData.damage: 0.5` (below) |
| Blanks | 2 per floor | — | PlayerStatsShared.prefab `NumBlanksPerFloor: 2`; [wiki Blank](https://enterthegungeon.wiki.gg/wiki/Blank) |

### Guns

"Fire rate" on the wiki is the **cooldown between shots in seconds**. For example, Marine Sidearm DPS 14.5 = 10×5 / (9×0.25 + 1.2). Shot speed is in tiles/s.

| Gun (owner) | Clip | Reload | Cooldown → shots/s | Damage | Shot speed | Spread | Ammo | Source |
|---|---|---|---|---|---|---|---|---|
| Marine Sidearm (Marine) | 10 | 1.2 s | 0.25 → 4.0 | 5 | 25 | 5° | ∞ | [wiki](https://enterthegungeon.wiki.gg/wiki/Marine_Sidearm) |
| Rogue Special (Pilot) | 8 | 1.2 s | 0.25 → 4.0 | 5 | 22 | 10° | ∞ | [wiki](https://enterthegungeon.wiki.gg/wiki/Rogue_Special) |
| Budget Revolver (Convict) | 5 | 0.9 s | 0.15 → 6.7 (semi-auto) | 6 | 23 | 10° | ∞ | [wiki](https://enterthegungeon.wiki.gg/wiki/Budget_Revolver) |
| Rusty Sidearm (Hunter) | 6 | 1.2 s | 0.20 tapping / 0.25 held → 5 / 4 | 6 | 16 | 7° | ∞ | [wiki](https://enterthegungeon.wiki.gg/wiki/Rusty_Sidearm) |
| Crossbow (Hunter, 2nd) | 1 | 0.75 s | 0.50 | 22 | 26 | 7° | 100 | [wiki](https://enterthegungeon.wiki.gg/wiki/Crossbow) |
| AK-47 | 30 | 0.5 s | 0.11 → 9.1 | 5.5 | 23 | 4° | 500 | [wiki](https://enterthegungeon.wiki.gg/wiki/AK-47) |
| Magnum | 6 | 1.0 s | 0.15 tap / 0.25 held | 13 | 23 | 7° | 140 | [wiki](https://enterthegungeon.wiki.gg/wiki/Magnum) |

**How reloading works** (code, [Gun.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Gun.cs)):
- **Manual reload** is a button, available any time (PlayerController.cs ~l.8425).
- **Auto-reload:** when the clip is empty, the next fire attempt calls `AttemptedFireNeedReload()` → `Reload()` (Gun.cs l.3048, l.3350). If you're holding fire, the reload starts immediately on empty. One-round-clip guns reload as soon as the shot animation ends (l.3324, l.1741).
- **Active reload** is *not* base behaviour. It is unlocked by the item Cog of Battle: press during the reload at the 2/3 mark to finish instantly, for +75% damage on that clip. Missing lengthens the reload ([wiki Cog of Battle](https://enterthegungeon.wiki.gg/wiki/Cog_of_Battle)).

### Enemies

All values are from the enemy prefabs in `SRC/Asset_Bundles/enemies_base_001/data/enemies/`. Details:
- HP is the base value. It scales up by floor and difficulty, e.g. the wiki lists Bullet Kin at 15 → 27.75 ([wiki](https://enterthegungeon.wiki.gg/wiki/Bullet_Kin)).
- Bullet speed is in tiles/s. The shotgun scripts use `Speed(5f)`, and BulletScript bullets move `Velocity/60` per 1/60 s tick, so these are also tiles/s ([Bullet.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Brave/BulletScript/Bullet.cs) l.500–508, [BulletScriptBehavior.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/BulletScriptBehavior.cs)).

| Enemy | HP | Move | Attack | Cooldown | Clip / reload | Bullet speed | Range | Prefab |
|---|---|---|---|---|---|---|---|---|
| Bullet Kin | 15 | 2 tiles/s | 1 aimed bullet (Magnum, gun id 38), no lead | 1.6 s | 6 / 2.0 s | 8 | 12 | [BulletMan.prefab](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/Asset_Bundles/enemies_base_001/data/enemies/BulletMan.prefab) |
| Bandana Bullet Kin | 15 | 2 | Machine Pistol (id 43) stream | 0.15 s | 20 / 2.0 s | 8 in its bullet bank (**unverified** whether the gun's own projectile overrides it) | 12 | [BulletManBandana.prefab](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/Asset_Bundles/enemies_base_001/data/enemies/BulletManBandana.prefab) |
| Red Shotgun Kin | 30 | 2 | 5 pellets, aimed, 6° apart | 3.5 s | 1 / 3.0 s | 5 | 20 | [BulletShotgunMan.prefab](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/Asset_Bundles/enemies_base_001/data/enemies/BulletShotgunMan.prefab), [RedBasicAttack1.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/BulletShotgunManRedBasicAttack1.cs) |
| Blue Shotgun Kin | 40 | 2 | 5 pellets at 20° spacing, then after 40 frames (0.67 s) 4 more at 20°, re-aimed if the player moved >30° | 4.0 s | 1 / 2.0 s | 5 | 20 | [BulletShotgunMan_Blue.prefab](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/Asset_Bundles/enemies_base_001/data/enemies/BulletShotgunMan_Blue.prefab), [BlueBasicAttack1.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/BulletShotgunManBlueBasicAttack1.cs) |

Other enemy notes (all from src prefabs):
- Contact damage for these enemies is 0.5 (`CollisionDamage`).
- Bullet Kin approach until in range (`SeekTargetBehavior`, `CustomRange: 7`) and will take cover behind or flip tables (`TakeCoverBehavior`).
- **Jammed** ("Black Phantom") variants get 0.66× cooldown, 1.5× move speed and 1.0× bullet speed.

**Telegraphs.** `ShootGunBehavior` has no built-in wind-up.
- The only telegraph is an optional `enemyPreFireAnimation` on the enemy's gun, whose length becomes the pre-fire time, or an optional laser sight (`PreFireLaserTime`).
- All four enemies above have `PreFireLaserTime: -1` (no laser).
- I did **not** verify whether the Magnum, Machine Pistol or shotguns carry a pre-fire animation ([ShootGunBehavior.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/ShootGunBehavior.cs) l.109–166, 389–397).
- In practice, the readable cue is the visible gun pointing at you plus slow bullets. There is no long flash.

### Blanks and tables (for scope)

- **Blank:**
  - Erases all enemy bullets in the room and briefly stops enemies firing.
  - Pushes back and deals 10 damage to enemies within 7 tiles.
  - Opens secret walls.
  - You get 2 per floor ([wiki Blank](https://enterthegungeon.wiki.gg/wiki/Blank)).
  - In code, `ForceBlank` defaults to radius 25 and 0.5 s at max radius ([PlayerController.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/PlayerController.cs) l.5622).
  - Losing armor fires one for free.
- **Table flip:**
  - Flipping clears nearby bullets during the flip animation. The flipped table then absorbs bullets as destructible cover.
  - Rolling over a table is a "slide", which lets you fire mid-slide. You can do it 3 times before the table breaks.
  - Bullet Kin flip tables too ([wiki Table](https://enterthegungeon.wiki.gg/wiki/Table)).

### Room and camera scale

- **Camera:**
  - The native buffer is 480 × 270 px at 16 px/tile, i.e. **30 × 16.9 tiles visible**, landscape ([Pixelator.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Pixelator.cs) l.273–275; [BraveCameraUtility.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/BraveCameraUtility.cs) l.190–194).
  - The camera leads toward the aim point and can zoom in some situations. I did not measure those offsets.
- **Rooms:** I read `m_width`/`m_height` from all 252 room templates in `shared_auto_002/data/rooms/1_castle/` (Keep of the Lead Lord, floor 1). These are my own stats over those files; whether the template size includes wall cells is unverified.
  - **NORMAL (combat) rooms, n=222:**
    - width p10/p25/**p50**/p75/p90 = 17/20/**24**/26/30
    - height = 15/18/**20**/23/26
    - smallest 10×10, largest 40×40
  - **HUB rooms, n=24:** median 32×30, up to 40×40.
  - A typical combat room is therefore about 0.8 screens wide and a bit over 1 screen tall.

## Per-topic notes

**Movement.**
- 7 tiles/s with zero acceleration. The "slow" feel comes from low top speed, not from inertia.
- Moving is one of three things you do. Rolling is barely faster than walking. Stopping costs nothing.

**Dodge roll.**
- ETG's roll is a **commitment**: 0.65 s with locked direction, no shooting, and the last ~45% vulnerable.
- It is not an escape dash. It is for passing *through* a bullet line during the first 0.36 s.
- Because the roll's average speed ≈ walk speed, "roll to go faster" isn't a strategy. That is what keeps rolls meaningful.
- Our roll (850 pt/s for 0.2 s, fully invulnerable, re-available 0.4 s later) is a dash in comparison.

**Guns and reloads.**
- Starter clips of 5–10 at 4–6.7 shots/s empty in about 1–2.5 s, followed by about 1 s of reload.
- That produces the burst → reposition → burst rhythm.
- The reload still ticks during a roll (per the code), so "reload, then roll" is the natural combo. The roll's no-shoot window overlaps the reload's no-shoot window.

**Health.**
- Half-heart damage on 3 hearts = 6 hits.
- After a hit you're untouchable for 1.0 s, and bullets pass through you for 1.4 s. That generous window lets you escape the bullet cluster that hit you.

**Enemies.**
- Basic enemies are slow walkers: 2 tiles/s, 0.29× player speed.
- Their bullets are about player speed: 5–8 tiles/s.
- Pressure comes from bullet count and angles, not from bullet speed.

## Translation guide (1 tile = 32 pt)

| Parameter | ETG → our units | Ours today | Delta |
|---|---|---|---|
| Player move | 7 tiles/s → **224 pt/s** | 420 pt/s | ours 1.9× faster |
| Roll distance | 5 tiles → **160 pt** | 170 pt | ≈ same distance |
| Roll duration | **0.65 s** | 0.2 s | ours 3.25× shorter |
| Roll avg speed | 7.7 tiles/s → **246 pt/s** (peak ≈343, tail ≈139) | 850 pt/s | ours is a dash |
| Roll i-frames | first **0.36 s** (≈118 pt of travel), last 0.29 s vulnerable | full 0.2 s | — |
| Roll cooldown | **0** after the roll ends (cycle ≈0.65 s) | 0.4 s after a 0.2 s roll (cycle 0.6 s) | similar cycle |
| Shoot during roll | no | ? | — |
| Starter gun | 4 shots/s, 10 clip, **1.2 s** reload (≈2.9 shots/s sustained) | 7.5 shots/s, no reload | ours ≈2.6× sustained fire rate |
| Starter damage vs Bullet Kin | 5 dmg vs 15 HP = **3 shots/kill** | ? | — |
| Player bullet | 25 tiles/s → **800 pt/s** | 900 pt/s | close |
| Enemy bullet (Bullet Kin) | 8 tiles/s → **256 pt/s** | 300 pt/s | ours 1.17× |
| Enemy bullet (Shotgun Kin) | 5 tiles/s → **160 pt/s** | — | — |
| Enemy bullet / player speed | **1.14** (Bullet Kin), 0.71 (Shotgun Kin) | 0.71 | at 224 pt/s player + 300 pt/s bullets → 1.34 |
| Player bullet / enemy bullet | 3.1 | 3.0 | same |
| Basic enemy walk | 2 tiles/s → **64 pt/s** | rusher 180 pt/s (5.6 tiles/s) | no ETG basic enemy is a rusher. At 224 pt/s player, a 180 pt/s rusher is 0.8× player speed (vs ETG's 0.29×) |
| Shooter cadence | Bullet Kin **1.6 s**, reloads 2 s every 6 shots | 1.6 s | identical |
| Shooter telegraph | none built in (gun aim only; pre-fire anim unverified) | 0.6 s | ours is more telegraphed |
| Player HP | 3 hearts, half-heart hits = **6 hits** (+1 armor Marine) | 5 whole-heart hits | ETG slightly more |
| Post-hit protection | **1.0 s invuln + 1.4 s bullet pass-through** | 0.75 s | ETG ≈2× longer |
| Visible area | 30 × 16.9 tiles → **960 × 540 pt** (landscape) | iPhone portrait ~393 × 852 pt ≈ **12.3 × 26.6 tiles** | we see ~40% of ETG's width |
| Combat room | median 24 × 20 tiles → **768 × 640 pt** | ? | — |

**Portrait readability.**
- ETG gives about 8.4 tiles from the player to the top or bottom edge. A Bullet Kin bullet crosses that in about 1.05 s.
- On a portrait phone at 32 pt/tile there are about 6.1 tiles to the side edge. Our 300 pt/s bullet crosses that in about 0.65 s.
- Matching ETG's reaction time on the short axis means slower bullets (≈190 pt/s), a smaller tile, or camera lead toward threats.

**A literal port for a first tuning pass:**
- Player 224 pt/s; roll 160 pt / 0.65 s with i-frames for the first 0.36 s; no roll cooldown; no firing while rolling.
- Starter gun 4 shots/s, 10 rounds, 1.2 s reload.
- Player bullet 800 pt/s; enemy bullet 256 pt/s (Bullet Kin) or 160 pt/s (shotgun pellets).
- 6 half-heart hits; 1.0 s invulnerability + 1.4 s bullet pass-through.

Then adjust for the portrait screen.

## Unverified / caveats

- **Source vintage.** The data comes from a community decompile/asset export (pushed 2022-10), not from Dodge Roll. The values are the game's own serialized data, but I didn't confirm which game version it is.
- **Gun stats** are wiki infobox values. I didn't cross-check them against gun prefabs.
- **Reload continues during a roll.** Inferred from code (no roll check in `HandleReload`). Not observed in game.
- **Enemy telegraphs.** Whether the Bullet Kin's Magnum or the Shotgun Kin guns have an `enemyPreFireAnimation` (a wind-up) was not checked.
- **Bandana Bullet Kin bullet speed.** Its bullet bank says 8, but it doesn't pin a bullet name, so the gun's own projectile may be used instead.
- **Shotgun Kin effective cadence.** Clip 1 with a 3.0 s reload alongside a 3.5 s cooldown. The exact interaction isn't worked out.
- **Room sizes.** Whether template dimensions include walls is unknown. Only floor 1 (Keep) was sampled.
- **Camera.** Dynamic camera lead and zoom offsets were not measured.
- **Roll speed profile.** The early 10.7 / late 4.3 tiles/s figures are my Hermite evaluation of the serialized animation curve.
- **Other characters.** All player numbers are for the Marine. Other characters share the same stats prefab (move 7, 2 blanks). Starting health and armor may differ per character. For example, the Marine prefab has `currentHealth: 4`, which looks like a stale editor value; the wiki says 3 hearts.
