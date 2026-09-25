# How does Enter the Gungeon generate a floor?

Research ticket. Question: how does ETG turn hand-made rooms into a floor (flows, room tables, layout, seeding, guarantees)? We want the same thing for derelicts: a seeded, deterministic generator in `crates/sim` that assembles our `PrototypeRoom`s (`crates/sim/src/room.rs`) into a floor, replacing the handmade five-room line in `crates/sim/src/derelict.rs`.

Researched 2026-09-25.

**Sources.**
- **`src`**: the community decompile / asset export [fedes1to/EtG-source](https://github.com/fedes1to/EtG-source), the same snapshot `etg-combat-feel.md` used (pushed 2022-10). Generator code lives in `Assembly-CSharp/Dungeonator/` plus a few top-level classes. Flow graphs are serialized `.asset` files in `ExportedProject/Assets/MonoBehaviour/F*_Flow_*.asset`. I read the C# and parsed all 32 main-floor flow assets. This is the highest-trust source, but **it is reverse-engineered**: names are real, the comments are gone, and I can't tell which game version it is.
- **`boris`**: [Boris the Brave, "Dungeon Generation in Enter The Gungeon" (2019)](https://www.boristhebrave.com/2019/07/28/dungeon-generation-in-enter-the-gungeon/). A community code walkthrough. It agrees with `src` except on one retry count (noted below).
- **`dev`**: Dodge Roll's own words. Dave Crooks in [MCV/DEVELOP (2016)](https://mcvuk.com/development-news/random-mechanics-and-the-art-of-being-predictably-unpredictable/) and [N4G (2015)](https://n4g.com/user/blogpost/indiemonth/534329). Brent Sodman quoted on the [official wiki's Save Button page](https://enterthegungeon.wiki.gg/wiki/Save_Button).
- **`mod`**: the [EtG Modding Guide, "Making the flow"](https://mtgmodders.gitbook.io/etg-modding-guide/making-a-floor/making-the-flow) and [GungeonSeedMod](https://github.com/ApacheThunder/GungeonSeedMod). These are secondary and only used to corroborate.

`src` links below are relative to `github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/`.

## TL;DR

- **Floors are hand-authored graphs that get filled with hand-made rooms.** Each floor has 4–8 hand-drawn **flows**. A flow is a small directed graph with 16–25 nodes. Each node is a room *category* (normal, hub, reward, boss, connector, special), not a specific room. Generation picks a flow, picks a concrete room for each node from a weighted **room table**, **injects** extras (secret rooms, NPC cells, shrines), and then lays the rooms out in space. Crooks: rooms are "randomly joined together following a set of rules … that we believe describe good dungeon design" (`dev`).
- **The shipped flows are almost fixed.** In all 32 main-floor flows, no node has `percentChance < 1` and no node is optional. Only 2 flows use a selector/subchain node, and single-node "expanding chains" (`n → nn`, length 2–3) appear only in Sewers, Mines, Hollow, Forge and Bullet Hell flows. **Nearly all the variety comes from which flow is picked, which rooms fill it, injections, and geometry.** The graph grammar exists but is barely used.
- **Guarantees live in the flow data, not in checks.** Every flow has exactly 1 BOSS node, 2 REWARD (chest) nodes (3 in two Mines flows), and a shop node plus a boss-foyer node (both SPECIAL). Secret rooms, NPCs and shrines are probabilistic **injections** on top.
- **Layout is exit-pairing plus backtracking, with corridors only for loops.** A new room is attached to a placed room by lining up a pair of facing exits. The code tries 0–3 cells of extra hallway on each side, checks cell collisions, and backtracks on failure. A loop is grown from both ends and then closed with a pathfound corridor (4–30 cells) or a procedural filler room. Failures retry: 5–100 times per sub-graph, 50 times per flow, then it switches to a new flow.
- **Seeding is simple, but ETG's determinism is leaky.** Generation uses one `System.Random(seed)` with float draws, plus `UnityEngine.Random` in a few places. Results also depend on save data (unlocks, and "recently seen" memory for flows, rooms and bosses). Dodge Roll's programmer said he wasn't "a hundred percent certain" floors were deterministic (`dev`). **We should copy the structure and avoid those inputs.**
- **The biggest simplification for us is that our rooms are separate spaces.** Doors link room-local coordinate spaces (`Derelict::link`), so we don't *need* ETG's hardest part: spatial layout, collision and corridors. A graph-only generator (flow → rooms → exit pairing) gives ETG's floor *structure* at a fraction of the code. Spatial embedding is only needed for a geometric minimap or a continuous world (see Open questions).

## 1. Dungeon flows

### Data model (`src` [DungeonFlow.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/DungeonFlow.cs), [DungeonFlowNode.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/DungeonFlowNode.cs))

A `DungeonFlow` (ScriptableObject) has:
- `m_nodes` and `m_firstNodeGuid` (the root). Edges are `childNodeGuids` (a tree) plus at most one `loopTargetNodeGuid` per node (extra edges that make cycles), with a flag `loopTargetIsOneWay`.
- `fallbackRoomTable`: the floor's room table, used by any node without an override.
- `subtypeRestrictions`: per-floor caps. For example, every Keep flow caps NORMAL/TRAP rooms at 1 per floor.
- `flowInjectionData` / `sharedInjectionData`: the injection sets (§4).

A `DungeonFlowNode` has:

| Field | Meaning |
|---|---|
| `nodeType` | `ROOM`; `SELECTOR` (pick `min..max` of its children, weighted by their `percentChance`); `SUBCHAIN` (splice in a random sub-flow rooted at a node whose `subchainIdentifier` matches, with a copy limit) |
| `roomCategory` | CONNECTOR, HUB, NORMAL, BOSS, REWARD, SPECIAL, SECRET, ENTRANCE, EXIT |
| `percentChance` | chance this child is built at all (all shipped flows use 1) |
| `priority` | MANDATORY / OPTIONAL. An optional node that gets no room is skipped instead of failing |
| `overrideExactRoom` / `overrideRoomTable` | pin one room (entrance and exit elevators) or one table (shop, boss foyer) |
| `nodeExpands` + `chainRules`, `min/maxChainLength` | string-rewrite grammar. Start from `"n"`, apply weighted regex rules like `n → nn` until the length is reached, then map each char to a category (`c n b r s h t e x`). One flow node becomes a chain of rooms |
| `forcedDoorType` | NONE / LOCKED / ONE_WAY on the edge to the parent |
| `isWarpWingEntrance` | the subtree is built as a separate, disconnected area reached by a warp |

### What the shipped flows look like (parsed from `src` `.asset` files)

Keep (`F1_Castle_Flow_01`, 16 nodes). `→` is a tree edge and `⤺` is a loop edge:

```
entrance(exact) → n → n → HUB ─┬→ n → REWARD ⤺ one-way back to the first n
                               └→ n ─┬→ HUB ─┬→ n → n → REWARD ⤺ one-way back to that HUB
                                     │       └→ n → SHOP(table)
                                     └→ FOYER(table) → BOSS → exit(exact)
```

Across all 32 main-floor flows:
- **Size.** Keep flows have 16–18 nodes. Gungeon Proper has 18–19, Mines 19–25, Hollow 19–21, Forge 21–23 and Bullet Hell 21. These counts come before injections and before chain expansion.
- **Fixed structure.** Each flow has 1 BOSS, 2 REWARD (3 in `Mines_02`/`Mines_03`), 1–2 HUB, 1–6 CONNECTOR, and 7–15 NORMAL rooms.
- **SPECIAL nodes** are the shop plus the boss foyer. Forge has just 1 SPECIAL node, an exact room.
- **Loops.** Flows have 0–3 loop edges. **Most are one-way edges from a REWARD dead end back to an earlier hub.** This is a shortcut that saves backtracking after you grab a chest, and it's the pattern worth stealing.
- **Names describe the designer's intent**: `TwoPathsToBoss`, `QuickBoss`, `FourSmallWings`, `SnowflakeLoopBranches`, `LoopSeries_ExitThroughGiftShop`, `Maze`. Each flow is a hand-designed floor *shape*.
- **Counts per floor.** Keep has 6 flows, Gungeon Proper 8, Hollow 4, Mines 5, Forge 5. `boris` also counts Hollow = 4 and Gungeon Proper = 8.

### How a flow is chosen

- [`GameLevelDefinition.LovinglySelectDungeonFlow`](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/GameLevelDefinition.cs) filters the floor's `flowEntries` by prerequisites. Flows with a nonzero "differentiator" (seen recently) go to a separate list, which is only used if nothing else is left. The pick is then weighted by `weight` (default 1). An entry marked `forceUseIfAvailable` wins outright.
- How the differentiator works ([GameStatsManager.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/GameStatsManager.cs) `EncounterFlow`, ~l.798): generating a flow adds +2 to its differentiator, and each finished run subtracts 1. **So a flow you just saw is avoided for about 2 runs.** This is cross-run memory kept in the save file.
- If the whole flow fails to lay out after 50 attempts, `LoopDungeonGenerator` picks another flow and tries again, unless the flow was force-assigned. The pick uses `SemioticDungeonGenSettings.GetRandomFlow`, which is uniform and also avoids recently seen flows.

## 2. Room tables and picking a room for a node

**Tables** ([GenericRoomTable.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/GenericRoomTable.cs), [WeightedRoom.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/WeightedRoom.cs)):
- A table is a list of `WeightedRoom { room, weight, limitedCopies, maxCopies, additionalPrerequisites }` plus nested `includedRoomTables`, flattened into one list.
- **One table per floor holds every category.** `Castle_RoomTable` has 298 rooms, all at `weight: 1`, 297 of them limited to 1 copy, and it includes the secret-room table. The node's category is the filter. In practice **weights are flat, so a room table is a set of rooms and the category is the query.**

**Picking** (`src` [LoopFlowBuilder.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Dungeonator/LoopFlowBuilder.cs) `GetViableAvailableRooms` / `GetAvailableRoom`, ~l.732–855). The candidates are the table's rooms (shuffled with the generation RNG) that pass every filter:
- `room.category == node category`.
- **`room.exits.Count >= node.Connectivity`**, where connectivity = parent + children + loop edge. A hub node needs a room with enough doors.
- A 1-exit NORMAL node never gets a TRAP-subcategory room, so trap rooms are never dead ends.
- Not already used on this floor (unless `ForceAllowDuplicates`), and under `maxCopies`.
- Passes room and entry prerequisites (unlocks, story flags) and injection flags.
- Not excluded by an already-placed room's `excludedOtherRooms`.
- Under the flow's `subtypeRestrictions` cap.

Then:
- **The weight is scaled by `clamp01(1 − 0.33 · roomDifferentiator)`** for recently seen rooms (not SPECIAL). This is cross-run memory again.
- The pick is a weighted draw.
- If nothing fits there is a fallback ladder:
  1. Allow duplicates.
  2. Ignore `maxCopies`.
  3. CONNECTOR/HUB degrades to NORMAL.
  4. SECRET may return `null`, which silently drops the room.

Special cases:
- **Connectors are picked lazily during layout** (`AcquireRoomDirectionalIfNecessary` in [LoopBuilderComposite.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Dungeonator/LoopBuilderComposite.cs)). By then the parent is placed, so the pick is filtered to rooms with an exit facing one of the parent's *unused* exits.
- **Boss foyer.** `HandleBossFoyerAcquisition` re-picks the node before BOSS so the foyer has an `EXIT_ONLY` exit facing the boss room's entrance.
- **Boss.** A BOSS node with no override calls [`BossManager.SelectBossRoom`](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/BossManager.cs). That is a weighted pick among the floor's bosses, where the last boss seen on that floor gets ×0.5 weight.

**Exits** ([PrototypeRoomExit.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/PrototypeRoomExit.cs)):
- Each exit has `direction`, `exitType` (`NO_RESTRICTION` / `ENTRANCE_ONLY` / `EXIT_ONLY`, which maps to our `ExitKind::{Either, Entrance, Exit}`), `exitGroup` (A–H), `containsDoor` and `exitLength`.
- **Only one exit per group may be used.** This lets a designer offer "north door *or* east door" as alternatives.
- **Unused exits are never opened.** `RoomHandler` stamps only `instanceUsedExits`, so an unused exit stays wall. **So in ETG, exits are optional door slots, not required links.** Our `Derelict` validation currently requires every exit to be linked exactly once. That has to change (see build order).

## 3. Layout (`src` `LoopFlowBuilder.DeferredBuild`, `LoopBuilderComposite`, `SemioticLayoutManager`)

Pipeline, in order:

1. **Compose the tree** (`ComposeFlowTree`). Instantiate flow nodes into `BuilderFlowNode`s. Selectors, subchains, `percentChance` and chain expansion are resolved here. Loop edges become `loopConnectedBuilderNode` links.
2. **Acquire rooms** for every node (§2). Then fix up the boss foyer.
3. **Inject** extra nodes (§4). This rewires the tree.
4. **Re-root at the node with the highest connectivity**, usually a hub, so building starts from the most connected room.
5. **Split into composites** (`ConvertTreeToCompositeStructure`). Each loop, found as the simplest cycle through the loop edge, becomes one *loop composite*. The loop-free fragments become *tree composites*, and warp wings get their own.
6. **Build each composite on its own canvas**, starting at (0,0):
   - **Tree composites: depth-first with backtracking.** For each child, list candidate `(parentExit, childExit)` pairs:
     - **Facing pairs** (E–W, N–S) come first, shuffled.
     - **Perpendicular "jointed" pairs** come after. These get an L-shaped hallway with 3 extra cells, and they are disallowed next to SECRET and BOSS rooms.
     - Pairs are ordered to **prefer exits far from the parent's already-used exits**, which spreads branches out.
     - Exit groups and `ENTRANCE_ONLY`/`EXIT_ONLY` are respected.
   - **Placement test** (`CanPlaceRoomAtAttachPointByExit`):
     - Put the new room where its exit meets the parent's exit. Try hallway extensions of 0–3 cells on each side, 16 combos walked diagonally, so shorter total length is tried first.
     - Broadphase: bounding boxes padded by 1 cell left/bottom, 2 right and 4 top (room for face walls).
     - Narrowphase: the room's cells, including face walls, against the occupied-cell set.
     - If a child's subtree fails, try the next exit pair. If all pairs fail, the node fails.
   - **Loop composites: grow two arms, then close.**
     - Place the first node. Alternately place the next node on the "left" arm (list index 1, 2, …) and the "right" arm (index n−1, n−2, …).
     - After the first quarter of the loop, exit pairs are **sorted to minimize the Manhattan distance between the two arm tips.**
     - Close the gap with `AttemptLoopClosure`. If the gap rectangle is roomy (w, h > 6, one side > 12, area < 350, aspect 0.2–5), stamp a **procedural rectangular room**. Otherwise **A\* a hallway** that must be 4 to < 30 cells long (50 in the Mines).
     - One-way loop edges become one-way doors.
   - **Retry.** A failed composite re-acquires its rooms (new random picks) and rebuilds: **up to 100 times for a loop, 5 for a tree.** `boris` says "up to 3 times", which may be a different version.
7. **Merge canvases.** Breadth-first from composite 0, larger composites first:
   - To attach a new canvas, try exit pairs between the external and internal node and search nearby offsets for a spot where the whole canvas fits (`FindNearestValidLocationForLayout`).
   - If both ends of an edge are already placed, **pathfind a hallway between them** (`ConnectTwoPlacedLayoutNodes`).
   - Any attach failure fails the whole build.
8. **Whole-build retry.** Up to 50 fresh `LoopFlowBuilder` attempts per flow. The RNG keeps advancing, so each attempt differs. After that, switch flows and loop forever ([LoopDungeonGenerator.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Dungeonator/LoopDungeonGenerator.cs)).
9. **Finalize.** Offset everything to positive coordinates. Mark cells within about 7 of any room as dungeon cells. Find entrance and exit by category. BFS `distanceFromEntrance` for every room.

**Takeaways:**
- Almost all of the complexity serves the *spatial* problem: fitting fixed-shape rooms into one grid without overlap, and closing loops geometrically.
- The *graph* problem is small: choose a flow, fill nodes by category and degree, pair exits.

## 4. Injection: secrets, shops, NPCs, and guarantees

**Guaranteed content is in the flow.** The boss, 2 chest rooms, the shop and the foyer are all flow nodes. Chests come from REWARD rooms. The shop is a SPECIAL node whose table is the shop table. The modding guide builds custom floors the same way (`mod`).

**Probabilistic content is injected** ([ProceduralFlowModifierData.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Dungeonator/ProceduralFlowModifierData.cs), `LoopFlowBuilder.ProcessSingleNodeInjection`, ~l.276). Each modifier has:
- an exact room or a table;
- `chanceToSpawn`;
- prerequisites;
- a list of `placementRules`, one picked uniformly;
- `chanceToLock` and `OncePerRun`.

Placement rules:

| Rule | Where the new node goes |
|---|---|
| `END_OF_CHAIN` | child of a random leaf. The leaf can't be EXIT or SECRET, and can't have a two-way loop. Falls back to `RANDOM_NODE_CHILD` |
| `RANDOM_NODE_CHILD` | child of a random standard-category node, optionally at a minimum depth from the entrance |
| `BEFORE_ANY_COMBAT_ROOM` | spliced between a combat room and its parent (not inside a two-way loop) |
| `COMBAT_FRAME` | brackets a run of N consecutive combat rooms with two rooms (start and end) |
| `HUB_ADJACENT_*`, `AFTER_BOSS`, `NO_LINKS` (warp-only), `BLACK_MARKET` | as named |

Concrete data (`src` assets):
- **Secret rooms** (`Base Shared Injection Data`, attached to every flow):
  - `chanceToSpawn 0.9`.
  - `placementRules [RANDOM_NODE_CHILD, END_OF_CHAIN ×4]`, so **80% hang off a dead end and 20% off any room.** This matches `boris`.
  - The same set has a "Loot Cell" at 10%.
- **Secret connection.** The link to a SECRET-category room gets flagged cells and a `SecretRoomBuilder` cover, which is the hidden wall you blank or shoot open ([RoomHandler.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Dungeonator/RoomHandler.cs) ~l.3464, 4328). The shoot-open behavior is general game knowledge; I did not trace it in code.
- **Keep-specific** (`Castle Common Injection Data`): the Oubliette (sewers) entrance, always spawned, `RANDOM_NODE_CHILD`, allowed to become secret. Also a "Fireplace Backup".
- **Run-level spreading.** [MetaInjectionData.PreprocessRun](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Dungeonator/MetaInjectionData.cs) runs once per run:
  - For each global set (NPC table, shrine table, sub-shop table, secret-plus table), it rolls how many appear this run (for example 1–2) and **shuffles which floors get them.**
  - It optionally adds a "bonus secret" copy (5%) that is forced into a secret room.
  - This is how ETG spreads shops and NPCs across a run instead of rolling each floor independently.
- **Room-required injections.** A placed room can require another injection (`requiredInjectionData`), for example a room that needs a partner room somewhere else.

## 5. Seeding and determinism

- **RNG.** [BraveRandom.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/BraveRandom.cs) is one static `System.Random(int)` behind `GenerationRandomValue()` (a `float` draw). `GenerationRandomRange(int,int)` = `floor((max−min)·value)+min`. `UnityEngine.Random` is `InitState`-ed with the same int and also used, for example for the flow pick and the meta-injection trigger rolls.
- **Seed per floor** ([Dungeon.cs](https://github.com/fedes1to/EtG-source/blob/main/ExportedProject/Assets/MonoScript/Assembly-CSharp/Dungeonator/Dungeon.cs) `GetDungeonSeed`, ~l.239):
  - In a seeded run, the floor seed is `CurrentRunSeed`, **the same int for every floor**. Floors differ only because their data differs.
  - Otherwise it's a random 1–10⁹.
  - Seeded runs are not exposed in the vanilla UI. `GungeonSeedMod` enables them (`mod`).
- **Why it's leaky.** Layout depends on things outside the seed:
  - unlock and story prerequisites on rooms, flows and injections;
  - NPC-cell pity (`NumberRunsValidCellWithoutSpawn / 50`);
  - the flow, room and boss "differentiators" in the save file.
  - Seeded runs set `IgnoreGenerationDifferentiator = true` to switch off the recently-seen memory. `GungeonSeedMod` still warns that layouts only match "as long as you've unlocked all the NPCs" (`mod`).
  - Sodman: "i'm not a hundred percent certain that i've made our floors deterministic yet" (`dev`, via the wiki).
- **What this means for us:** generation must be a pure function of `(floor definition, seed)`. Any meta-progression that changes the room pool must be an explicit, recorded input, not ambient save state.

## 6. Design sketch for our sim

Constraints we already have:
- `fixed` Fx math, no floats;
- `Rng` (xoshiro256\*\*, `below(n)`);
- `HashMap`/`HashSet` banned by clippy;
- `'static` rooms validated in `const fn`;
- `MAX_ROOMS = 64` (the cleared set is a `u64`);
- rooms are **separate spaces** joined by `Derelict::link`.

### Core decision: generate a graph, not a map

Since a door moves the party into another room-local space, **a floor is just rooms plus connections.** That is exactly `Derelict` minus `'static`. Skip ETG steps 5–9 (composites, canvases, collision, corridors). Loops become free: a loop is one more `Connection`. Keep the `Dir::faces` rule on links so the doors still read spatially (you leave east and arrive from the west). Spatial embedding can be added later as a *validator* for a minimap without changing the output type.

### Data types (sketch)

```rust
/// Static, const-validated content, like PrototypeRoom today.
pub struct RoomTable { pub rooms: &'static [WeightedRoom] }
pub struct WeightedRoom { pub room: &'static PrototypeRoom, pub weight: u16 } // integer weights

pub struct FlowNode {
    pub category: Category,
    pub pick: Pick,                   // Table (floor default) | Exact(&'static PrototypeRoom) | From(&'static RoomTable)
    pub parent: Option<u8>,           // node 0 is the entrance; tree by parent index
    pub loop_to: Option<(u8, OneWay)>,
}
pub struct Flow { pub name: &'static str, pub nodes: &'static [FlowNode] }

pub struct Injection {
    pub pick: Pick,
    pub chance_permille: u16,         // 900 = ETG's secret-room 90%
    pub placement: &'static [InjectAt], // uniform pick, e.g. [RandomChild, EndOfChain ×4]
}
pub enum InjectAt { EndOfChain, RandomChild { min_depth: u8 }, BeforeCombat }

pub struct FloorDef {
    pub flows: &'static [(&'static Flow, u16)], // weighted
    pub table: &'static RoomTable,
    pub injections: &'static [Injection],
}

/// Runtime output: owned, serializable, checksummable. Replaces `&'static Derelict` in SimState.
pub struct Floor {
    pub rooms: Vec<RoomKey>,          // index into one static ROOMS catalog (serializable, unlike &'static)
    pub connections: Vec<Connection>, // existing type; unused exits = sealed
    pub start_room: u16,
    pub start_cell: (u8, u8),
}
```

Everything is `Vec` or slices indexed by `u8`/`u16` and iterated in index order. There are no maps. Weights are integers and draws use `Rng::below(total)`.

### Generation steps: `fn generate(def: &FloorDef, seed: u64) -> Floor`

1. **Seed.** Use `Rng::from_seed(floor_seed)`, where `floor_seed` is derived from the run seed and the floor index (e.g. `next_seed` chained per floor). Don't reuse the same seed on every floor the way ETG does.
2. **Pick a flow**, weighted.
3. **Instantiate nodes.** Copy the flow nodes into a `Vec<Node { category, parent, children, loop_to, room: Option<RoomKey> }>`. There is no chain expansion or selectors in v1, since ETG barely uses them.
4. **Inject.** For each injection, roll `below(1000) < chance_permille`. Then pick a placement rule and collect valid target nodes in index order. Pick one and append a child node. A secret room is just `category: Secret` plus `EndOfChain`/`RandomChild`.
5. **Assign rooms and exits together**, in BFS order from the entrance, with backtracking. For each node:
   - Build candidates from its table: category matches, `exits.len() >= degree`, not used yet on this floor, and has a *free* exit that faces the parent's chosen exit with a compatible `ExitKind`.
   - Draw by weight, then choose the exit pair (prefer the parent exit farthest from its used ones, as ETG does).
   - On dead end, pop back and try the next candidate, with a bounded budget, e.g. 64 steps per node.
6. **Close loops.** For each `loop_to`, pick a free facing exit pair between the two rooms. If none exists, retry step 5 for that loop's nodes or drop the loop edge. That fork is below.
7. **Retry or fall back.** If the budget is exhausted, restart from step 2 with the same `Rng`, which has advanced so the retry is still deterministic. Allow N attempts (e.g. 20), then **fall back to a known-valid handmade floor** (today's `DERELICT`). The player should never see a generation failure.
8. **Validate** (a runtime `fn validate(&Floor) -> Result<(), FloorError>`, reusing `DerelictError` where possible):
   - The room count is ≤ `MAX_ROOMS`.
   - Each exit is linked at most once, and links face each other with equal width.
   - Every room is reachable from the start.
   - Exactly one Entrance and at least one Exit room exist.
   - Every flow-mandated category count is present. The Exit is reachable.
   - A one-way loop never strands the party: after taking it, the Exit must still be reachable.

### Tests (load-bearing, not exhaustive)

- **Seed sweep:** `generate` + `validate` for seeds `0..N` against every `FloorDef`, where N is a few thousand. This one test catches thin room pools and bad flows.
- **Golden:** a fixed seed gives a fixed checksum of `Floor`. This guards replay/rollback determinism, the same way `rng.rs` pins its reference values.

### Content implication

ETG's Keep pulls from about 300 rooms. The generator is only as good as the pool: each category needs enough rooms, **with exits on varied sides**, and hubs need 3–4 exits. Otherwise step 5 hits dead ends constantly. Plan for about 3× the flow's node count per category before judging the generator.

### Build order (smallest first)

1. **`Floor` runtime type.** `DERELICT` converts into it and `SimState` holds it (or holds seed + def and rebuilds it; see below). **Unlinked exit cells render and collide as wall.** This is the one sim change the generator needs. Nothing gets generated yet.
2. **Linear generator.** One hard-coded flow: `Entrance → Normal×k → Exit`. Room and exit pairing come from a small pool (the current 5 rooms plus a few with other exit sides), with the seed sweep and golden tests. This is already a playable "random derelict".
3. **Tree flows.** Hubs, branches and REWARD dead ends. Multiple weighted flows per `FloorDef`.
4. **Loops.** One-way shortcut from a reward back to a hub, which only needs one extra connection plus a one-way door flag on `Connection`.
5. **Injections.** Secret room first (it needs a "hidden door" exit state), then optional rooms (shop, terminals).
6. **Only if needed: spatial embedding** for a minimap. Place room bounding boxes on an integer grid by exit pairing with collision, reusing the ETG algorithm in §3 without corridors.
7. **Probably never:** chain grammar, selectors/subchains, cross-run "recently seen" weighting. If the last one is ever added, it must be an explicit recorded input.

## Open questions for Jeff

1. **Graph-only or spatial?** Graph-only is much simpler. It has no geometry, and loops come free. Costs: a minimap can only be an abstract node map, and geometric contradictions are possible, e.g. a loop whose rooms couldn't physically coexist. Spatial embedding makes the ship feel physically consistent and supports an ETG-style map. It roughly triples generator complexity and needs more exit variety in rooms. Recommendation: graph-only now. Revisit when the minimap design exists.
2. **Where does the floor live in `SimState`?** One option stores the generated `Floor`: a few hundred bytes in snapshots, checksummed. The other stores only `(floor_def, seed)` and regenerates on load. Regenerating is smaller and trusts determinism. Storing is simpler to reason about for rollback and late-join resync. Recommendation: store it, since generation runs once per floor and not per tick.
3. **One-way doors and locked doors.** ETG's best loop pattern is the one-way shortcut from a reward room back toward a hub. Adopting it means a door state on `Connection` (one-way, locked, secret). Decide whether that fits the derelict fiction (bulkheads, vents, hull breaches) before step 4.
4. **Do exits stay optional door slots?** This is ETG's model: rooms author more exits than any single layout uses, and unused ones seal. The alternative is to keep "every exit linked exactly once", which makes pairing much harder and forces a tailor-made room per graph shape. Recommendation: optional slots. This reverses one of the current `Derelict` validation rules.
5. **Fallback policy.** On repeated failure, either use a handmade floor silently or log it in dev builds and treat it as a content bug. The seed sweep should make fallback unreachable in practice, but something must happen if it isn't.

## Jeff's decisions (2026-09-25)

1. **Graph-only for now.** We do want a minimap later, so keep room placement recoverable. Don't design anything that rules out embedding the graph in space.
2. **Store the generated `Floor` in `SimState`.**
3. **No one-way doors, and no locked doors yet. Secret doors: yes, definitely.** `Connection` needs a door kind for secret.
4. **Exits become optional door slots.** Unused exits seal.
5. **Generation must not fail.** No fallback path. Make failure unrepresentable or impossible by construction, and prove it with the seed sweep.

## Unverified / caveats

- **Source vintage.** All code and asset facts come from the 2022-10 community decompile. The retry counts (5/100 per composite, 50 per flow) disagree with `boris`'s "up to 3 times". Either version differences or a misread are possible.
- **Loop closure details.** My description of the procedural-room-vs-hallway choice and the arm-growing order comes from reading `LoopBuilderComposite.BuildLoopComposite`/`AttemptLoopClosure`. I did not run it.
- **Flow parse.** Node categories, loop edges and counts come from a quick YAML parse of the `.asset` files. Room/table GUIDs were not resolved to names, except the injection sets. I identified "shop" and "boss foyer" from the SPECIAL+table position in the graph and the foyer-acquisition code, not from resolving the table GUIDs.
- **Secret-door mechanics.** The claim that the secret link is a breakable wall you blank or shoot is general game knowledge. I didn't trace it in code beyond the `isSecretConnection` flag and `SecretRoomBuilder`.
- **Chest counts.** "2 REWARD nodes per flow" is a flow-data fact. The real chest count per floor also depends on reward-room contents and random chest drops, which I did not research.
- **Interview quotes.** The MCV quote was fetched and verified. The N4G quote ("Each room is hand made, but the floors are procedurally assembled from a huge pool of rooms") comes from a search summary and was not re-read at the source.
- **Alternative approach.** [Edgar-Unity's ETG example](https://ondrejnepozitek.github.io/Edgar-Unity/docs/next/examples/enter-the-gungeon/) recreates Gungeon-style floors with graph-based layout (configuration spaces). It's worth a look if we ever go spatial. I didn't evaluate it.
