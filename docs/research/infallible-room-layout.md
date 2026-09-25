# How do we place rooms so layout cannot fail?

Research ticket. Question: how do we place hand-authored rooms (`crates/sim/src/room.rs`) into **one contiguous tile grid**, joined by short generated hallways, such that generation **cannot fail for any seed**? Follows `etg-level-generation.md`, whose decisions (2026-09-25) this doc takes as fixed: exits are optional door slots and unused ones seal, the generated `Floor` lives in `SimState`, there are no one-way or locked doors, secret doors are required, and there is no fallback path.

Researched 2026-09-25.

**Constraints restated.**
- Rooms are hand-authored cell grids (floor/wall/pit/void) with 2-wide exits on their bounding-box edges (validation already enforces this).
- Rooms are placed by pairing *facing* exits (E–W, N–S).
- Hallways are generated. They are 1–4 cells long, at least 2 wide, straight when the exits line up and jogged when offset. No room touches another room directly.
- The sim is deterministic: integers only, seeded xoshiro `Rng::below`, `Vec`/`BTreeMap` iterated in index order.
- The flow is an authored graph of categories: airlock ×3, combat, reward, secret, bridge boss.

**Sources.**
- **`src`**: the community decompile [fedes1to/EtG-source](https://github.com/fedes1to/EtG-source). I re-read `Dungeonator/SemioticLayoutManager.cs`, `LoopBuilderComposite.cs`, `LoopFlowBuilder.cs`, `LoopDungeonGenerator.cs` and `RuntimeRoomExitData.cs` / `PrototypeRoomExit.cs` for this doc. **Reverse-engineered, version unknown.**
- **`boris`**: Boris the Brave, [ETG writeup (2019)](https://www.boristhebrave.com/2019/07/28/dungeon-generation-in-enter-the-gungeon/) and [Binding of Isaac writeup (2020)](https://www.boristhebrave.com/2020/09/12/dungeon-generation-in-binding-of-isaac/).
- **`edgar`**: Ondřej Nepožitek's blog ([Basics](https://ondra.nepozitek.cz/blog/graph-based-dungeon-generator-basics-1/), [Implementation](https://ondra.nepozitek.cz/blog/graph-based-dungeon-generator-implementation-2/)), [Edgar-Unity docs](https://ondrejnepozitek.github.io/Edgar-Unity/docs/introduction/) and its [ETG example](https://ondrejnepozitek.github.io/Edgar-Unity/docs/examples/enter-the-gungeon/), and [Edgar-DotNet docs](https://ondrejnepozitek.github.io/Edgar-DotNet/docs/introduction/). The algorithm comes from Nepožitek & Gemrot, "Fast configurable tile-based dungeon level generator" (Game-ON 2018), which builds on Ma et al. 2014. **I read the blog and docs, not the thesis PDF.**
- **`spelunky`**: Darius Kazemi, [Spelunky Generator Lessons](http://tinysubversions.com/spelunkyGen/), an interactive reconstruction and not Mossmouth's code.
- **mine**: the recommendation's invariant and proof sketch are my own derivation. No source proves them, so check them in review and in the tests described below.

## TL;DR

- **Nobody in the survey is infallible for arbitrary flows.** ETG retries up to 50 builds per flow and then switches flows. Edgar uses simulated annealing plus backtracking and documents that "some inputs are too hard". Isaac retries from scratch. Only fixed-lattice generators like Spelunky are infallible by construction, and they pay for it with uniform room sizes.
- **The 1–4 cell hallway cap is the binding constraint.** Classic collision-free tree layouts (tidy trees, slicing floorplans) work by pushing subtrees apart with arbitrarily long edges. A 4-cell cap forbids that, so for general trees a collision-free layout is not guaranteed to exist. **Infallibility has to come from restricting the flow's shape**, or from precomputing layouts.
- **Recommendation: a "spine and legs" layout** (a caterpillar tree, plus local "U" loops). The main path runs along one axis. Each spine room can hang one leg (a chain of rooms) off its north side and one off its south side. Each room, hallway and leg gets its own **disjoint region of the plane**, so overlap is impossible whatever the rooms or seed are. Generation is `fn generate(&FloorDef, u64) -> Floor`, with no `Result`, no retry and no backtracking. It is O(rooms).
- **What makes it infallible:** a small set of room-pool contracts that are checked when the crate builds, plus a region-ownership invariant (proof sketch in §4). A seed-sweep test and an adversarial-pool test check the *stamped grid* independently of the construction.
- **What it costs:**
  - Floors have a comb silhouette: a long keel with compartments off it. This suits a derelict ship.
  - Loops only span one spine gap.
  - Leg rooms must be no wider than the spine room they hang from.

## 1. How the surveyed approaches work

### ETG: `LoopFlowBuilder` + `SemioticLayoutManager` (`src`, `boris`)

- **Attach.** A child is placed so its exit's origin coincides with the parent exit's origin. `GetExitOrigin(TotalExitLength)` pushes each origin outward by the exit length plus `additionalExitLength`.
  - `CanPlaceRoomAtAttachPointByExit` tries extra lengths of 0–3 on each side. It walks 16 combos along diagonals so shorter totals come first.
  - **Facing exits are always collinear. ETG has no lateral jog.** The only non-straight joins are perpendicular "jointed" pairs, which get +3 cells and an L bend.
  - **Our jog is a degree of freedom ETG doesn't have**, and the recommendation below relies on it.
- **Collision.**
  - Broadphase: room bounding boxes padded −1/+2 in x and −1/+4 in y (room for face walls), plus a set of "exit test points".
  - Narrowphase: `GetCellRepresentationIncFacewalls()` against a `HashSet<IntVector2>` of occupied cells.
- **Search.** Tree composites are built depth-first with backtracking over exit pairs, preferring exits far from the parent's used ones.
- **Loops.** The two arms of a loop are grown alternately, sorted to close the Manhattan gap between them. `AttemptLoopClosure` then either stamps a procedural rectangle room (w, h > 6, one side > 12, area < 350, aspect 0.2–5) or A\*-pathfinds a "phantom corridor" 4 to < 30 cells long (50 in the Mines and Office).
- **Failure.**
  - A composite is rebuilt with fresh room picks: 100 times for a loop, 5 for a tree. `boris` says 3.
  - `LoopDungeonGenerator` makes up to 50 full `LoopFlowBuilder` attempts, and the caller then switches flows.
  - **Probabilistic, with no bound on attempts.**
- **Pool demand.** Rooms need `exits.Count >= connectivity`, and their exits must face varied directions so that some facing pair exists at each attach. With about 300 rooms per floor, this rarely bites.
- **Determinism.** It mostly works: a seeded `System.Random` with float draws, and `Mathf.PingPong`/`RoundToInt` in the diagonal walk. `HashSet` is only used for membership and "any" tests, so iteration order doesn't leak. The 30 ms time-slicing only yields and doesn't affect results. The leaks come from save-data inputs (see the earlier doc), not from layout.
- **Cost.** Roughly 5k lines across the four files, most of it spatial search and corridor pathfinding.
- **What to take from it:** exit-pairing attachment and the "prefer exits far from used ones" heuristic. **Leave behind:** the backtracking and the retries.

### Edgar: graph-based layout with configuration spaces (`edgar`)

- **Input.** A level graph plus room templates (an outline polygon and door positions). Corridors are handled by inserting a corridor node between every pair of neighbors, with its own templates.
- **Configuration space.** For a fixed room A and a free room B, the configuration space is the set of B positions that don't overlap A and connect to A through a door. These are precomputed per template pair and used to propose moves.
- **Chains.** The graph is planar-embedded and decomposed into chains, **faces (cycles) first**, then the acyclic remainder. Each chain is laid out by simulated annealing: move or reshape one node, with an energy that fines overlap and disconnected neighbours. Partial layouts sit on a stack for backtracking.
- **Loops.** First-class. This is the method's selling point: "something you usually don't see in graph-based generators".
- **Infallible?** No. Failure is an empty stack, and the docs say "some inputs are too hard for the generator" and "usually works best for levels with less than 30 rooms". Planarity is required. The platformer variant swaps annealing for greedy search, which makes it faster but no more complete.
- **Determinism.** Annealing uses float temperatures and acceptance probabilities. A port would need fixed-point energy and integer acceptance thresholds. That's doable, but a long tail of iteration-count sensitivity.
- **Pool demand.** Moderate: templates need enough door positions for configuration spaces to intersect. Edgar's ETG example uses six room types plus corridor rooms.
- **Cost.** High: polygon configuration spaces, planar embedding, chain decomposition, annealing and backtracking. It is the right tool if arbitrary loopy graphs are non-negotiable. **The idea worth stealing is the configuration space:** for our rectangles and 2-wide exits, "B relative to A through exit pair (e, f)" is just `gap ∈ 1..=4 × offset ∈ −J..=J`. That's the vocabulary §3 uses.

### Retry-from-scratch grid growth: Binding of Isaac (`boris`)

- **How it works.** Breadth-first growth on a 9×8 cell grid from the start cell. It refuses to add a cell that already has two neighbours, which is why Isaac has no loops apart from the secret room. Rooms are 1×1, 2×1, 2×2 or L.
- **Failure.** If the room count is wrong or the boss is adjacent to the start, it **retries from the start**.
- **Verdict.** Probabilistic, and it needs lattice-sized rooms. Not for us.

### Fixed lattice with signature templates: Spelunky (`spelunky`)

- **How it works.** A 4×4 grid of equal-size rooms. A random walk from the top row goes left, right or down until it leaves the bottom row. Each cell's template *type* is forced by the openings the walk needs, for example "type 2 always has a bottom drop".
- **Infallible by construction.** The walk always terminates, and every required opening signature has templates. (This is my reading. `spelunky` doesn't discuss failure because there is nothing to fail.)
- **Cost to us.** Every room would have to be one of a few fixed sizes (within the 4-cell hallway slack of a lattice pitch), with exits at fixed edge positions. That discards the variety of hand-drawn room shapes.

### Precomputed layout tables / skeletons (offline solve, runtime fill)

- **How it works.** Offline, run any solver (ETG-style search, Edgar, a SAT/ASP solver) per flow to produce N **skeletons**. Each skeleton assigns every node a slot: a bounding box, fixed exit cells per side, and the hallways between them. At runtime, pick a skeleton by seed, then fill each slot with any pool room whose footprint and exits fit the slot within the hallway slack.
- **Infallible?** Yes, provided every skeleton was verified offline and every slot class has at least one room (checkable at compile time).
- **Loops.** Arbitrary. The solver pays the cost offline.
- **Pool demand.** High. Rooms must be authored to slot classes. Variety ≈ skeletons × fill choices. Silhouettes repeat every N floors unless N is large.
- **Cost.** An offline tool, a checked-in data format, and a regeneration step whenever rooms or flows change. Determinism at runtime is trivial.
- **Verdict.** A good *escape hatch* for set-piece floors or big loops. It's heavy as the main path.

### Constraint solvers at runtime (ASP/SAT)

Answer-set programming and SAT generators (for example Smith, Padget & Vidler 2018) are *complete*, not infallible: UNSAT is still a failure, and solve time is unbounded. They are only viable as the offline solver above. **Not evaluated in depth.**

### Summary table

| Approach | Loops | Pool demand | Infallible? | Determinism risk | Cost |
|---|---|---|---|---|---|
| ETG exit-pairing + backtracking | two arms + pathfound corridor 4–30 | ~300 rooms, varied exit sides | no, retries up to 50× then switches flow | low (float draws) | high |
| Edgar configuration spaces | first-class, planar only | door positions per template | no, "too hard" inputs, < 30 rooms | medium (annealing floats) | very high |
| Isaac grid growth | none (secret room only) | lattice-sized rooms | no, retries from scratch | low | low |
| Spelunky lattice | implicit via grid | fixed sizes and exit slots | **yes** | none | low |
| Precomputed skeletons | arbitrary (offline) | slot-class conformant rooms | **yes** (if verified) | none | medium–high plus tooling |
| **Spine and legs (recommended)** | local U-loops | width + exit-side contracts | **yes, by region ownership** | none | low |

## 2. Why general trees can't be guaranteed with ≤ 4-cell hallways

- **Tidy-tree and slicing-floorplan layouts avoid overlap by construction.** They compute each subtree's bounding box bottom-up and place sibling boxes side by side. That forces the parent-to-child edge to stretch across the siblings' boxes, which can be arbitrarily long.
- **With edges capped at 4 cells, siblings must pack tightly.** Whether they fit then depends on room sizes and exit positions. A hub with three long branches on fixed-size rooms can have no valid layout at all.
- **So the fix is not a smarter search. It's to restrict the flow.** Allow only shapes whose regions *can't* interfere given the hallway rule.
- **One property rescues us: the jog gives unbounded lateral freedom.** A hallway is short along its axis, but its offset across the axis is free. The recommended shape spends all of its "unbounded" needs on that lateral axis.

## 3. Recommendation: spine and legs

### Shape

```
            [L]        [L]                [L]
             |          |                  |
            [L]  ┌─U─┐ [L]                [L]
             |   |   |  |                  |
 [airlock]==[S1]===[S2]===[S3]====[S4]====[bridge boss]
             |          |          |
            [L]        [L]        [L]
                        |
                      [secret]
```

- **Spine.** An ordered list of rooms, placed monotonically along +x. Each consecutive pair is joined E→W by a spine hallway.
- **Legs.** Each spine room may have one **north leg** and one **south leg**. A leg is a chain of rooms stacked away from the spine, joined N–S. Legs don't branch.
- **U-loop.** An optional extra hallway between the tips of the north legs (or the south legs) of two *adjacent* spine rooms. It turns spine gap *i* into a two-path section.
- **Fit to our flow (my mapping).** The airlocks can be legs (hull doors) or spine ends. Combat and reward rooms go on the spine or legs. Secrets go at leg tips. The bridge boss ends the spine.
- **Fit to ETG.** ETG's Keep flow is *close* to a caterpillar but not exact: its second hub hangs a two-branch subtree off the main path. Expressing it needs a minor re-route (putting that hub on the spine). ETG's shortcut loops span several rooms; ours are local.
- **Variety knobs** (none of them affect the proof):
  - leg lengths;
  - the lateral jog of every hallway;
  - room choice;
  - leg-room lateral position within its column;
  - **horizontal mirroring**: flipping a 3/4-view room left–right is safe, while a vertical flip isn't;
  - running the spine along y instead of x, since the proof is symmetric.

### Hallway rule (the configuration space)

A hallway joins exit `a` (on room A's edge) to facing exit `b` (on room B's edge) across a gap of `L` cells between the two edges. Let `d` be the offset of `b` relative to `a` along the edge.

- **Straight:** `d == 0` and `L ∈ 1..=4`. Floor is 2 wide, with wall rows either side for the gap's length.
- **Jog (Z):** `d != 0` and `L == 4`. The cells are: a 1-cell stub, a 2-wide leg running across the axis for `|d|` cells, and a 1-cell stub. Walls sit on the gap's first and last lines and past the leg's ends. **With L = 4, every floor and wall cell of the Z lies inside the gap strip, for any `d`.** It needs no room wall as a flank. That is why jogs use the full 4.
  - Jogs with L = 2–3 are possible, but only where both rooms' walls flank the whole leg, meaning their spans along the edge overlap. That's an optional polish rule and it complicates the proof. Skip it in v1.
- **Result:** a hallway is fully determined by `(a world cells, b world cells, L)`. It's a pure function, so it can be unit-tested per shape.

### Data sketch

```rust
/// Authored, 'static, validated in const fn like rooms. The shape IS the type:
/// a flow that isn't spine+legs+U-loops is unrepresentable.
pub struct Flow {
    pub name: &'static str,
    pub spine: &'static [SpineNode],   // index 0 = entrance end, last = boss end
}
pub struct SpineNode {
    pub slot: Slot,
    pub north: &'static [Slot],        // leg, nearest-first; empty = none
    pub south: &'static [Slot],
    /// U-loop to the next spine node's leg tip on this side (both legs non-empty).
    pub loop_next: Option<Side>,       // Side::{North, South}
}
pub struct Slot { pub category: Category, pub pick: Pick } // Pick as in etg-level-generation.md

/// Runtime output, stored in SimState (decision 2). Small: no dense grid.
pub struct Floor {
    pub rooms: Vec<PlacedRoom>,        // index = RoomId
    pub halls: Vec<Hall>,
    pub start: (RoomId, (u8, u8)),
}
pub struct PlacedRoom { pub key: RoomKey, pub origin: (i32, i32), pub mirrored: bool }
pub struct Hall { pub a: ExitRef, pub b: ExitRef, pub len: u8, pub offset: i16, pub door: DoorKind }
pub enum DoorKind { Plain, Secret }
```

- **The dense tile grid is derived, not stored.** `FloorGrid::bake(&Floor)` stamps rooms and then halls into a `Vec<Cell>` over the bounding box, together with a parallel `Vec<Option<RoomId>>` for "which room is this cell". It's a cache, rebuilt on load and not part of the checksum.
- **Size bound.** The worst case is 64 × 68 ≈ 4.4k cells per axis. Realistic floors are a few hundred cells a side.

### Algorithm: `fn generate(def: &FloorDef, seed: u64) -> Floor`

There is no `Result`, no retry and no backtracking. Every step picks from a set that §4 proves non-empty.

1. `rng = Rng::from_seed(seed)`. Pick a flow by integer weight. Roll injections (for example the secret room). An injection **appends a slot to a leg**, or creates a one-room leg on an empty side of a spine node. Targets are gathered in index order, so the set is deterministic. It is non-empty whenever the flow has at least one spine node with a free side or at least one leg, which is a compile-time check per flow.
2. **Derive each slot's exit signature.** Spine *i* needs W (unless first), E (unless last), plus N or S if it has those legs. A north-leg room *k* needs S, plus N if *k* isn't the tip, plus E or W if it's a U-loop end. Include the `ExitKind` direction: spine W is Entrance/Either, E is Exit/Either, and so on.
3. **Pick spine rooms**, then **leg rooms filtered by `width ≤ anchor spine room's width`**. Candidates are pool rooms with a matching category and exits covering the signature. Prefer rooms not yet used on this floor, **but allow reuse** when the preferred set is empty. Uniqueness must never be the thing that empties a set. Draw with `rng.below(total_weight)`.
4. **Place the spine.**
   - Spine 0 at the origin.
   - For each next room, pick exits `(E of i, W of i+1)`, a gap `L ∈ 1..=4`, and an offset `d`:
     - if the gap has a U-loop: `d = 0`;
     - otherwise `d = 0` or `d ∈ −J..=J` with `L = 4`.
   - Room i+1's origin follows from those choices.
5. **Place legs.** For north leg rooms outward from the spine:
   - Choose the room's x so it lies inside spine room *i*'s x-span. A U-loop end goes **flush** to the span's edge nearest the gap.
   - Choose a gap `L` (straight if the exits line up and `L ∈ 1..=4`, otherwise a Z with `L = 4`).
   - Place it above the previous room. South legs mirror this.
6. **U-loops.** Join the two leg-tip rooms' E/W exits across the spine gap, which was given `L = 4` in step 4. The hallway is straight if their y's match, a Z otherwise.
7. **Emit the `Floor`.** Normalize origins so the minimum is (0, 0). Unused exits seal (decision 4). Secret-room halls get `DoorKind::Secret`.

## 4. Why it can't fail

### Part A: every choice set is non-empty (compile time)

Checked by a `const fn` over `(pool, flows)`, the same way `.valid()` works today, so a violation is a build error:

1. For every slot in every flow, at least one pool room matches the slot's category and exit signature. That includes injected slots and loop ends.
2. **Width contract.** For every leg slot, the narrowest matching room is no wider than the narrowest room matching its anchor spine slot. So whatever spine room is drawn, at least one leg candidate fits.
3. Exits never include a bounding-box corner cell. This keeps a hallway's end walls (exit ± 1) inside the owning region. **This is a new `RoomError`.** Today's validation allows corner exits.
4. Every injection has a non-empty target set in every flow it applies to.
5. Rooms have ≤ `MAX_SIDE` cells per side (already checked), and flows have ≤ `MAX_ROOMS` rooms including injections.

Given these, steps 1–3 of the algorithm always have a candidate. Steps 4–6 always have at least one legal parameter: the straight or `L = 4` option.

### Part B: nothing overlaps (region ownership)

Proof sketch (mine). Write spine room *i*'s bounding box as `[a_i, b_i] × [top_i, bot_i]`. Partition the plane:

- **Room boxes** `B_i`, owned by spine room *i*.
- **Gap strip** `G_i = [b_i+1, a_{i+1}−1] × ℤ`, owned by spine gap *i*'s hallways. It exists because `L ≥ 1`, and the strips are disjoint because the spine is strictly monotone in x.
- **Columns** `N_i = [a_i, b_i] × (−∞, top_i)` and `S_i = [a_i, b_i] × (bot_i, ∞)`, owned by spine room *i*'s legs.

These are pairwise disjoint by definition. Then:

1. **Leg rooms stay in their column.** Their width is ≤ `b_i − a_i + 1` (contract 2), and the placer puts them within `[a_i, b_i]`. They stack with ≥ 1 row between them. A leg hallway occupies only the rows between its two rooms, at x-cells between its two exits ± 1. Contract 3 keeps those x's inside `[a_i, b_i]`. So everything in `N_i` belongs to one chain and is ordered by y.
2. **Spine hallways stay in their strip.** A straight or `L = 4` Z hallway's cells all lie in `G_i`, whatever `d` is (hallway rule). Its y-range is `R_s = [min(Y_i, Y_{i+1}) − 1, max(Y_i, Y_{i+1}) + 2]`, where Y is each exit's top cell.
3. **A U-loop hallway also lies in `G_i` and is disjoint from `R_s`.**
   - The loop's ends are flush: A at `b_i`, B at `a_{i+1}`. So its horizontal extent is exactly `G_i`.
   - Its exits satisfy `y_A + 2 ≤ top_i − 1`, because A sits ≥ 1 row above spine room *i* and its exit is inside its box. Likewise `y_B + 2 ≤ top_{i+1} − 1`.
   - Loop gaps force `d = 0`, so `Y_i = Y_{i+1} = Y`. Also `Y ≥ top + 1` for both rooms, since the exit is off the corner.
   - So the loop's lowest cell, `max(y_A, y_B) + 2`, is ≤ `Y − 2`, which is strictly above `R_s`'s top at `Y − 1`. A south loop is the mirror image, so each gap can hold both a north loop and a south loop.
   - **This is the only step that needs a special case** (`d = 0` on loop gaps). Without it a tall neighbour could put `R_s` level with a leg tip.
4. **Everything is disjoint.** Every stamped cell falls in exactly one owner's region, except exit cells, which both a room and its hallway write as floor. So no two stamps conflict. ∎

### Part C: the rest of the guarantee

- **Connectivity** is by construction. The spine plus legs form a spanning tree, and every tree edge gets a hallway.
- **Termination and cost.** One pass over at most `MAX_ROOMS` nodes, with a constant number of RNG draws per node.
- **Determinism.** Integer math and `Vec`s in index order. The only randomness is `rng.below`. No floats and no maps.

### Proving it in tests (load-bearing only)

- **Seed sweep.** For every shipped `FloorDef` and seeds `0..10_000`:
  - `bake()` the grid with a **checking stamper** that panics on any write conflict;
  - assert every hallway has `len ∈ 1..=4`, a width of 2, and is straight or an `L = 4` Z;
  - flood-fill walkable floor from the start and assert it reaches every room;
  - assert the category counts match the flow.
  This tests the *output*, not the construction's own bookkeeping.
- **Adversarial pool.** The same checks over randomly *generated* pools: random room sizes up to `MAX_SIDE`, random exit positions, random voids, filtered only by the Part A contracts. This is what backs "for any pool that passes the build", which is a stronger claim than "for our 20 rooms".
- **Golden.** A fixed seed produces a fixed `Floor` checksum. This guards replay and rollback.

## 5. Build order (smallest first)

1. **Contiguous floor from today's slice.**
   - Add `Floor` and `FloorGrid::bake`. Give the handmade `DERELICT` hand-written origins, joined by straight hallways.
   - Switch collision, encounters and seal/unseal from per-room `Tiles` to the world grid plus the cell → `RoomId` map. **This is the real sim change. Nothing is generated yet.**
   - Add the checking stamper and the flood-fill checks here.
2. **Hallway function.** Straight and `L = 4` Z shapes, with unit tests per shape.
3. **Spine-only generator.** Straight hallways, a small pool, and the Part A const checks for spine slots. Add the seed-sweep and golden tests.
4. **Spine jogs.** `d ≠ 0` with `L = 4`.
5. **Legs.** Add the width filter, contract 2 and the corner-exit `RoomError`. Add the adversarial-pool test here.
6. **Injections.** Secret room at leg tips or empty sides, with `DoorKind::Secret`.
7. **U-loops.**
8. **Variety.** Horizontal mirroring, a y-axis spine, short jogs flanked by room walls. Each must keep the Part B argument or come with its own lemma.

## Open decisions for Jeff

1. **Accept the comb shape, or lift the 4-cell cap?** Spine and legs is infallible *because* hallways are ≤ 4. It costs every floor's silhouette: one keel with perpendicular legs, where legs don't branch. The alternative is to allow long generated corridors (ETG's loop corridors run 4–30 cells) on some edges. That unlocks bounding-box tree layouts with arbitrary branching, still infallible, but floors get long connecting hallways.
2. **Are loops needed, and are local U-loops enough?** U-loops give "two ways past spine gap *i*". Loops spanning several rooms, like ETG's reward-to-earlier-hub shortcuts, aren't expressible. If you want those, the infallible route is **precomputed skeletons** for specific set-piece flows, which means offline tooling. The other option is to ship v1 with no loops at all.
3. **Width contract direction.** The recommendation is "legs no wider than their anchor". The alternative is size classes, e.g. spine rooms ≥ 20 wide and leg rooms ≤ 20. That's simpler to author against but constrains the pool harder. This decides how rooms get drawn, so it's yours.

## Unverified / caveats

- **ETG facts** come from a decompile of unknown version. The retry counts (5/100/50) disagree with `boris`'s "3". "No lateral jog" is my reading of `GetExitOrigin` + `CanPlaceRoomAtAttachPointByExit`. I didn't run it.
- **Edgar internals** (how corridor configuration spaces compose, and actual failure rates) come from the blog and docs only. I didn't read the thesis or the code. "< 30 rooms" and "too hard" are the docs' own words.
- **Spelunky and Isaac** are community reconstructions, not the developers' code.
- **The proof in §4 is mine and unreviewed.** The riskiest spots:
  - the corner-exit contract (does every wall cell of a leg Z really stay in its column?);
  - rooms whose exits sit on void-edged parts of their bounding box;
  - any future renderer that draws wall faces *outside* the cell grid. ETG pads +4 cells above rooms for this. If we add face walls, spine gaps and leg gaps need a minimum `L` that covers the face height, which erodes the 1–4 range.
- **The Keep-flow mapping** onto spine and legs is my own, from the flow diagram in `etg-level-generation.md`.
