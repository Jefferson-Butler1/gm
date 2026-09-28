# How should a derelict pay you for clearing it?

Research ticket for the **Items and rewards** map. The question: how do ETG, Nuclear Throne, FTL, Hades, Dead Cells and Isaac structure rewards, and what's the smallest version for us? "For us" means themed room rewards, enemy-dropped currency, EMPs (our blanks), and real contents for the placeholder chest from #37.

Researched 2026-09-27.

**Where we are today.** `Theme` is `Corridor | Airlock | Bridge | Cargo | Crew | Engineering` (`crates/sim/src/room.rs`). Medical and tech/sensors don't exist yet. `Category::Reward` and `Category::Secret` exist, but nothing in the sim drops anything. The player has 5 whole-hit HP (`player.rs`). There is no armor. The phase pistol never runs out: it vents and regenerates charges (`gun.rs`), so **there is no ammo economy**. Map #22 fixes three things: a single floor per run (multi-floor is out of scope), EMP input is "swipe the EMP button to throw", and secret panels expose one `reveal` entry point.

**Sources.**
- **`src`**: the ETG decompile [fedes1to/EtG-source](https://github.com/fedes1to/EtG-source). It's the same snapshot our other research docs use, and its game version is unknown. I read `RewardManager.cs`, `FloorRewardData.cs`, `Dungeonator/RoomHandler.cs` (`HandleRoomClearReward`), `Dungeonator/Dungeon.cs` (`InformRoomCleared`, `AssignCurrencyDrops`), and the tuned values in `Assets/Asset_Bundles/shared_auto_001/data/AAA_REWARD_MANAGER.asset`.
- **`etgwiki`**: the official ETG wiki ([Chests](https://enterthegungeon.wiki.gg/wiki/Chests), [Blanks](https://enterthegungeon.wiki.gg/wiki/Blanks), [Armor](https://enterthegungeon.wiki.gg/wiki/Armor), [Keys](https://enterthegungeon.wiki.gg/wiki/Keys), [Money](https://enterthegungeon.wiki.gg/wiki/Money), [Hegemony Credit](https://enterthegungeon.wiki.gg/wiki/Hegemony_Credit), [Shop](https://enterthegungeon.wiki.gg/wiki/Shop), [Quality/Magnificence](https://enterthegungeon.wiki.gg/wiki/Magnificence), [Items](https://enterthegungeon.wiki.gg/wiki/Items), [Synergies](https://enterthegungeon.wiki.gg/wiki/Synergies), [Coolness](https://enterthegungeon.wiki.gg/wiki/Coolness)).
- **`nt`**: [Nuclear Throne wiki, Pickups and Props](https://nuclear-throne.fandom.com/wiki/Pickups_and_Props) (wikitext read through the Fandom API).
- **`ftl`**: FTL wiki pages [Scrap](https://ftl.fandom.com/wiki/Scrap), [Stores and resources](https://ftl.fandom.com/wiki/Stores_and_resources) and [Rewards](https://ftl.fandom.com/wiki/Rewards), plus [Mike Hopley's scrap tables](https://mikehopley.github.io/ftl-scrap/).
- **`hades`**: [Hades wiki, Chambers and Encounters](https://hades.fandom.com/wiki/Chambers_and_Encounters).
- **`dc`**: Dead Cells wiki pages [Pickups](https://deadcells.fandom.com/wiki/Pickups) and [Stats](https://deadcells.wiki.gg/wiki/Stats).
- **`isaac`**: Isaac wiki, [Item Pool](https://bindingofisaacrebirth.fandom.com/wiki/Item_Pool).

## TL;DR

- **ETG splits the questions "*whether* you get a reward" and "*what* it is."** Whether: a room-clear chance starts at 1%, grows about 9.8% with every empty clear, caps at 80–85%, and resets on any drop. What: a flat table of pickups (blanks, keys, armor, hearts, ammo), with about 10% of drops being a chest instead. The big items come from **2 guaranteed chest rooms per floor, forced to be 1 gun + 1 item**.
- **ETG's currency is a floor budget, not a per-kill roll.** At generation, about 65 casings (normal distribution, minimum 40) are split across the floor's enemies in advance. That is deterministic and seedable, which is exactly what our rollback sim wants.
- **ETG's blank:** clears all bullets, briefly stops enemy fire, pushes back and deals 10 damage within 7 tiles, and opens secret walls. You get 2 per floor. **Every armor piece that breaks fires one for free.**
- **Nuclear Throne's lesson is adaptive drops.** Drop rates fall when your ammo is high, and more drops turn into medkits the lower your HP (0→66%). One chest of each kind sits on every level.
- **FTL already has our theme model.** Its event rewards are typed (Fuel, Missiles, Drone parts, Scrap, "Stuff") × tiered (Low/Medium/High). Swap "type" for our room theme and we're done.
- **Hades shows the reward on the door.** That fits us perfectly, because our theme is known at generation.
- **Recommended slice:** 7 steps, covered in §5. The currency is **Salvage**, from a floor budget. There are 4 themed rewards (Engineering → gun mod, Crew → heal, Cargo → salvage cache, Bridge → plating + mod). The EMP is a 2-per-floor swipe-throw that also fires when **plating** (our armor) breaks. Mods have 3 tiers, and a 3-slot Quartermaster terminal sells them.

## 1. Enter the Gungeon

**Item types** (`etgwiki` [Items](https://enterthegungeon.wiki.gg/wiki/Items)):
- **Guns.**
- **Passives:** always on.
- **Actives:** "Most active items recharge as damage is dealt, while others recharge on a timer." Some recharge per room cleared, and single-use actives stack.
- There's 1 active slot, and some items add more.

**Quality tiers and chests** (`etgwiki` [Chests](https://enterthegungeon.wiki.gg/wiki/Chests), [Magnificence](https://enterthegungeon.wiki.gg/wiki/Magnificence)):
- The tiers are D (brown) / C (blue) / B (green) / A (red) / S (black), plus the rare rainbow, synergy and glitched chests, and mimics.
- The chest color *is* the quality. That's a one-glance read of how good the loot inside is.
- Treasure-room chests take 1 key.
- Chests can spawn with a fuse. If you don't open one in time, it explodes and leaves nothing.
- Shooting a chest open gives worse or no loot (`src`: `ChestDowngradeChance 0.25`, `ChestJunkChance 0.45`, `ChestExplosionChance 0.1`).
- **Per-floor quality weights (`src` asset):**

  | Floor | D | C | B | A | S |
  |---|---|---|---|---|---|
  | Floor 1 (Keep) | 0.25 | 0.32 | 0.20 | 0.09 | 0.04 |
  | Forge/Hell | 0 | 0.10 | 0.425 | 0.35 | 0.125 |

  The wiki quotes 35% D on Floor 1, which doesn't match the asset. The wiki may be describing a different version or normalization. **Uncertain.**
- **Guarantees in code:**
  - 2 reward-room chests per floor. If one already rolled "item", the other is forced to "gun" (`RewardManager.GenerationSpawnRewardChestAt`).
  - At most one D chest per floor after the Keep, and D is removed from the roll once 2 D chests have spawned in the run.
- **Anti-luck ("Magnificence"):** each A/S item you own sharply raises the chance that the next A/S roll gets downgraded (0 → 0%, 1 → 79.8%, 2 → 95.5%). It's a hard brake on snowballing.

**Room-clear rewards and the pity counter** (`src`, `HandleRoomClearReward` / `InformRoomCleared`):
1. Chance = `clamp(1% + AdditionalChestSpawnChance, 1%, cap)` + coolness − curse. The cap is 85% on floor 1 and 80% after.
2. On a miss, `AdditionalChestSpawnChance += 0.085 × 1.15` (about 9.8% per empty room). **Any drop resets it to 0.** The wiki's Coolness page ("~9%, max 80–85%") agrees with this.
3. On a hit, ~10% of rewards are a chest (`PercentOfRoomClearRewardsThatAreChests: 0.1`; the class default and some wiki text say 20%). Otherwise it's a pickup. Of pickups, 1/1.15 ≈ 87% come from the floor's `SingleItemRewardTable` and the rest are hearts (90% half, 10% full).
4. **Ammo is a separate roll** (`FloorChanceToDropAmmo` is 6–9%) that can drop even when the reward roll misses.

The wiki says 10.64% of room-clear pickups are blanks.

**Currencies:**
- **Casings** come in 1/5/50 denominations (`etgwiki` [Money](https://enterthegungeon.wiki.gg/wiki/Money)).
- `AssignCurrencyDrops` rolls a floor total at generation: normal(65, σ15), clamped to [40, 250].
  - Bosses get `rand(5, total/4)`.
  - "Signature" enemies get up to 10 each, capped at half of what's left.
  - The remainder is sprinkled 1 coin at a time onto random regular enemies.
- **Regular enemies that hit your hearts drop nothing.** Armor hits don't count.
- **Keys:** you start with 1. Keybullet Kin always drop one, and the shop always stocks at least one.
- **Hegemony Credits** are the meta currency, mostly from bosses (2–5 per floor) and doubled for a flawless kill. They're spent in the hub on unlocks.

**Shop** (`etgwiki` [Shop](https://enterthegungeon.wiki.gg/wiki/Shop)):
- Bello's counter has 3 item slots, filled 95% / 50% / 20% of the time, and each slot has a 20% chance to become a pickup instead.
- A side table sells blanks, glass guon stones and maps. There's always at least one key.
- Floor 1 shop quality is D 60% / C 30% / B 10%. **S items are never sold.**
- If you steal and get caught, the shop is gone for the rest of the run.

**Blanks and armor:**
- A blank erases all bullets, briefly stops enemy fire, pushes back and deals 10 damage within 7 tiles, and opens secret walls in the room.
- You get at least 2 per floor. There's no cap.
- Blanks are sold in the shop and dropped by room clears.
- Armor absorbs exactly one hit of any size, is spent before hearts, and **"triggers the effect of a blank when broken"** (`etgwiki` [Armor](https://enterthegungeon.wiki.gg/wiki/Armor)). Our `etg-combat-feel.md` traced this to `OnLostArmor → ForceBlank()`.

**Synergies:** hand-authored item pairs grant an extra effect, shown by a blue arrow and the synergy name (`etgwiki` [Synergies](https://enterthegungeon.wiki.gg/wiki/Synergies)). Synergy chests (`GlobalSynerchestChance` 0.05 in the asset) roll toward completing one. I didn't trace the full loot-weighting logic.

**For us:**
- **Copy the split between "whether" and "what".** Theme decides *what*. A pity counter decides *whether*, if we want scarcity at all (see Q2).
- **Copy `AssignCurrencyDrops` almost verbatim.** A floor budget pre-assigned to enemies at generation is deterministic, seedable and easy to tune. The same seed means the same salvage.
- **Copy "armor break fires a blank".** It's free drama and makes armor feel special.
- **Skip keys, fuses, mimics, magnificence and synergies in the first slice.** With a few dozen items, "no duplicates" is enough of a brake (see Isaac).
- **Guarantee the chest room**, as ETG's flow does.

## 2. Nuclear Throne

All values from `nt`:
- **Rads are XP.** You level up at 60 × current level, pick 1 of 4 random mutations at the end-of-level portal, and reach level 9 plus an Ultra.
- **Dropped rads fade after 5–6 s.** This is a push to move forward instead of hanging back.
- **Every level places three chests, one of each kind:**
  - Ammo: twice an ammo pickup, for your primary.
  - Weapon: better the deeper you are.
  - Rad canister: 25 rads.
- **Pity is built into the chests:**
  - If you skip a rad canister, a Big one (45) appears next level.
  - Every skipped weapon chest adds 25% to a Large Weapon Chest next time, so 5 skips guarantees one.
  - Under 50% HP, the canister has a 50% chance to become a medkit.
- **Adaptive drops:**
  - Enemy drop rate scales with how full your ammo is: 50% per weapon by default, 80% when under 20% ammo, 15% when over 60%.
  - An ammo drop becomes a medkit with chance `66% × missing-HP fraction`.
  - Medkits never spawn at full HP.

**For us:**
- We have no ammo, so NT's ammo loop doesn't transfer.
- **Two ideas do transfer, and they're cheap:**
  1. **Need-weighted healing.** Weight heal drops by `missing_hp / MAX_HP`, and give nothing at full HP. That's one line of deterministic integer math.
  2. **Fixed "one of each" placement per floor.** It's a predictable baseline that's easy to reason about.
- Level-up mutations are a second progression axis. Leave them for a later map.

## 3. FTL

- **Scrap is the only currency** (`ftl` [Scrap](https://ftl.fandom.com/wiki/Scrap)). It buys store goods and hull repair, and outside stores it upgrades systems and reactor power. Selling gives half price.
- **The central tension is upgrade now vs save for a store.** System upgrade costs rise by level; for example, Shields go 20/30/40/60/80/100 after level 2 (Steam price guide, via search; unverified against game files).
- **Rewards are typed × tiered** (`ftl` [Rewards](https://ftl.fandom.com/wiki/Rewards)):
  - "Standard" is scrap plus 2 resources plus a ~3% chance of a bonus item.
  - "Stuff" is resources plus low scrap plus ~6% bonus.
  - "Fuel", "Missiles" and "Drone parts" are that resource plus scrap.
  - "Scrap only" is just scrap.
  - Tiers are Low/Medium/High/Random, and scrap scales with sector and difficulty.
  - The sector type restricts which items can appear (for example, crystal items only in crystal sectors).
- **Stores** (`ftl` [Stores and resources](https://ftl.fandom.com/wiki/Stores_and_resources)):
  - Unlimited hull repair, whose price rises with sector.
  - Limited fuel/missiles/drone parts at fixed prices (fuel is 3).
  - 2–4 sections (weapons, drones, augments, crew, systems) of 3 items each, with no duplicate items.
  - Systems you lack are guaranteed (shields, medbay).

**For us: a ship-themed reward table.** It's FTL's typed reward keyed by room theme.

| Theme | Reward type | Status |
|---|---|---|
| Engineering | Gun mod (salvaged parts) | exists |
| Crew (galley/quarters) | Heal (rations); stands in for medbay | exists |
| Cargo | Salvage cache (currency) + Quartermaster terminal | exists |
| Bridge | Plating + a guaranteed mod roll (boss prize) | exists |
| Medical | Heal / max-HP | future theme |
| Tech / sensors | Reveal panels / map intel | future: needs the minimap |

FTL's "guaranteed shields if you lack them" maps onto our shop: always stock a heal.

## 4. Hades, Dead Cells, Isaac

**Hades** (`hades`):
- "The reward for each chamber is visible on its entrance door, and is obtained after clearing the chamber."
- Blue laurels are meta rewards and gold laurels are run rewards. A skull means a harder room with a better reward.
- Charon's shop sells "any 3 of the other laurel rewards" for obols.

**For us:** our theme is static at generation, so **a reward icon on each hatch costs almost nothing** and turns routing into a choice. It works on mobile because it's a glance, not a menu. It pays off once a ship has branches.

**Dead Cells** (`dc`):
- Three stats (Brutality/Tactics/Survival) each add 15% damage per point to weapons of that color.
- Scrolls raise stats. Each biome has a *fixed count* of 2-colour and 3-colour scrolls, so power is guaranteed per biome while the choice stays yours.
- Gold drops from enemies and flies to you.

**For us:**
- The useful ideas are **guaranteed power per floor** (our chest room) and **auto-collect**. ETG also vacuums casings on room clear.
- On a phone, stopping to walk over coins mid-fight is bad. Magnetize salvage when the room unseals.
- Skip choose-a-stat menus in the first slice.

**Isaac** (`isaac`):
- Every room type has its own **item pool** (treasure, shop, boss, devil, secret…).
- Items have a weight (almost all 1).
- An item is removed from all pools once generated, even if you never pick it up.
- An exhausted pool falls back to the treasure pool, then to Breakfast.

**For us:** this *is* the theme→pool model. Use per-theme weighted pools with **remove-on-generate**. That gives no duplicates, a natural brake, and a trivially deterministic `Vec` in the sim.

## 5. Recommended first slice

**Currency: Salvage.** It's scavenged from a derelict and works as both noun and verb. Alternatives: **Chits** (Belter-ish scrip), **Slugs** (railgun ammo; also an ETG nickname for casings), **Scrap** (too FTL).
- Denominations are 1 and 5.
- **Floor budget à la ETG:**
  - Roll the total at generation (starting point: mean 60, spread ±15, min 40).
  - Give the bridge boss a fixed share. Scatter the rest across enemies.
  - Store it as `salvage: u8` on each spawned enemy.
- Drops magnetize to the nearest living player when the room unseals.
- It's in-run only. Nothing persists until multi-floor or meta exists.

**Themed room rewards:** each cleared combat room drops its theme's reward at the room's reward point.

| Theme | Drop | Notes |
|---|---|---|
| Engineering | 1 gun mod from the Engineering pool | tier rolled 70/25/5 |
| Crew | Heal +1 HP | NT-style: at full HP it becomes 5 salvage |
| Cargo | Salvage cache (10–15) | also hosts the Quartermaster terminal |
| Bridge | 1 Plating + 1 mod, rolled one tier up | boss reward |
| Airlock, Corridor | nothing | |

**EMP (our blank):**
- **Supply:** you board with 2. There's no cap, but the HUD shows up to 3 plus "+n". Refresh per floor is moot while runs are one floor long.
- **Input:** swipe the EMP button to throw. The pulse travels a short fixed distance (~3 cells, ~0.15 s) and detonates. A tap with no swipe detonates at your feet, so the panic case stays instant.
- **On detonation:**
  - Clear every enemy bullet in the room.
  - Stun every enemy in the room for about 1 s. Tune this; ETG only says "brief".
  - Knock back and deal 1 pistol hit to enemies within ~3 cells.
  - Call `reveal` on any Panel hatch within that radius.
- The radius is what makes the throw matter: you aim at a suspicious wall.
- **Plating** is our armor: one pickup absorbs one hit and is spent before HP. **When plating breaks, it fires a free EMP at your feet**, as ETG does.

**Chest contents (the #37 placeholder becomes real):**
- The chest room holds one chest. It's unlocked, since the room is the guarantee and there are no keys yet.
- It holds 1 mod from the general pool, rolled 50/40/10 across the three tiers, plus 5–10 salvage.
- The chest's tint shows the tier, the ETG colour read.
- **Tiers:** Standard / Military / Prototype. With about 8 mods, three is enough.

**Starter mod pool.** These are all passives and all tweak existing `GunTuning` / player fields. There are no actives; the EMP is our active.

| Mod | Tier | Effect |
|---|---|---|
| Extended cells | Standard | +1 charge |
| Coolant loop | Standard | faster regen |
| Tungsten slugs | Standard | +damage |
| Hull plating | Standard | +1 Plating (also sold in the shop) |
| Split emitter | Military | 2-shot spread |
| EMP capacitor | Military | +1 EMP now, and plating EMPs are bigger |
| Rail coil | Prototype | piercing shots |

**Minimal shop, the Quartermaster.**
- It's a terminal in the Cargo room, usable once the room is cleared.
- It has 3 fixed slots: Heal (15), EMP (20) and one rolled mod (Standard 30 / Military 45).
- Purchases are sim inputs applied on a tick, per `coop-ready-sim.md`.
- There's no stealing and no reroll.
- A 60-salvage floor buys roughly one mod or two consumables. That's the FTL "now or later" tension in miniature.

**Build steps (one map, 7 PRs):**
1. **Pickups + Salvage.**
   - A `Pickup { kind, pos }` entity.
   - The floor budget is assigned at generation.
   - Enemies drop their share on death, and drops magnetize on unseal.
   - A HUD hook for the count.
2. **EMP.** Inventory, swipe-throw input, and detonation: bullet clear, stun, knockback, `reveal`.
3. **Plating + heal.** Plating absorbs a hit and fires an EMP when it breaks. The heal pickup is need-weighted.
4. **Mods + pools.**
   - `Mod` enum with tier and effect on tuning.
   - Per-theme weighted pools with remove-on-generate.
   - The chest opens to a mod.
5. **Themed room rewards.** Theme → drop on clear, plus a hatch reward-icon hook for Jeff's UI.
6. **Quartermaster.** 3-slot terminal and buy input.
7. **Tuning pass.** A seed sweep that reports salvage, mods and heals per floor, then a Garold playtest.

## Open questions for Jeff

1. **Currency name?** *Rec:* Salvage.
2. **Does every room pay, or is there an ETG-style pity roll?** *Rec:* every combat room pays its themed reward in this slice. Floors are short and the playtests need signal. Add an ETG pity roll (1% +~10%/empty clear, reset on drop) only if rewards feel cheap.
3. **Reward icons on hatches (Hades)?** *Rec:* yes. Ship a hook now, since it's nearly free. It matters more once templates have branching routes.
4. **EMP throw semantics?** *Rec:* swipe to throw ~3 cells, tap for your feet. Bullet clear and stun are room-wide. Knockback, damage and panel reveal are radius-only at the detonation point.
5. **EMP supply?** *Rec:* 2 on boarding, no cap, and droppable from Cargo/Crew caches as a rare substitute. Plating breaks fire free ones.
6. **Keys and locked chests?** *Rec:* no keys in this slice. The guaranteed chest room plus secret panels already carry the "earned access" feel. Revisit keys alongside the locked doors map #22 put out of scope.
7. **Co-op: shared or per-player loot?** *Rec:* shared salvage wallet, per-pickup consumables (whoever grabs it), and mods go to whoever opens or buys. It's the simplest model that's still fair in rollback.
8. **Should enemies that hit you drop no salvage (ETG rule)?** *Rec:* not in the first slice. It's a good skill-expression knob for the tuning pass once drops exist.

## Uncertainties

- **ETG numbers come from a decompile of an unknown version.**
  - Chest-vs-pickup (asset 10% vs wiki 20%) and Floor 1 chest weights (asset 25% D vs wiki 35%) disagree between the asset and the wiki.
  - Treat the *shape* as solid and the numbers as approximate.
  - The Fandom Coolness page also gives a different pity formula; the wiki.gg version matches the code.
- **I didn't verify:** ETG shop prices, what locks non-D chests (prefab data), synergy-chest weighting, and FTL system upgrade costs (they come from a Steam guide).
- **Nuclear Throne values** are from the community wiki, not code.
- **Dead Cells** per-biome scroll counts change between patches.
- **All our prices, budgets and radii above are starting guesses to tune on Garold, not derived values.**
