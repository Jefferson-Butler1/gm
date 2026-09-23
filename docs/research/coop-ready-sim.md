# What does co-op-ready demand of the sim?

Research ticket. Question: which netcode model should the headless Rust sim be designed for, so that 2-4 player online co-op (phones on cellular/Wi-Fi) can be added later without a rewrite? And what must the sim obey from day one?

Researched 2026-09-22. Crate versions are from the crates.io API on that date.

## TL;DR

- **Design for deterministic predict/rollback (GGPO/GGRS-style), with inputs relayed between peers.** Lockstep is the same sim with the prediction window set to 0, so it comes free as a fallback. Keep the whole state serializable so that host-authoritative snapshots, late join and resync stay possible.
- **The sim must be bit-deterministic across CPUs.** Use fixed-point math, a seeded RNG stored in state, deterministic entity IDs and iteration order, and per-tick quantized inputs for N player slots. It must be able to save/restore/advance one tick without rendering.
- These constraints cost almost nothing on day one. They are very expensive to retrofit into a float-based, HashMap-ordered, "the player" singleton sim.
- **Main risk: re-simulation CPU cost on a phone with many bullets and 3 remote players.** Budget for it early with a rollback-every-tick test and a tick-cost benchmark.

## Comparison

| | Deterministic lockstep | Rollback (GGPO / GGRS) | Host/server-authoritative snapshot interpolation (+ local prediction) |
|---|---|---|---|
| What goes over the wire | Inputs only | Inputs only | Inputs up, world snapshots/deltas down |
| Cross-platform determinism | **Required**, bit-exact | **Required**, bit-exact | Not required (only "close enough" for the predicted local player) |
| Float vs fixed | Fixed (or float with strict discipline) | Fixed (or float with strict discipline) | Floats fine |
| RNG | Seeded, state inside the sim state | Same, and it must roll back too | Only the host needs it. Clients can't predict random outcomes |
| Input model | Per-tick input for every player, applied on the same tick | Same, plus prediction (repeat last input) for remote players | Timestamped commands to the host. The host applies them at its tick |
| Snapshot/rollback cost | None per tick (full state is needed only for late join/resync) | Save every tick; restore + re-simulate up to N ticks in one frame | The host serializes, delta-compresses and prioritizes the world for every client every send tick |
| Bandwidth vs entity count | Flat (scales with inputs) | Flat | Grows with entities. Many bullets hurt unless spawned as events |
| Feel under mobile jitter | Input delay = worst peer latency. **Stalls** when any peer is late | Local input is instant. Remote players snap on misprediction. Stalls only past the prediction window | Local player is instant. Enemies and bullets are shown ~100 ms+ in the past, so "I dodged but got hit" unless the owning client is given authority over its own player's hits |
| 3-4 players | Worse: waits on the slowest of N | Worse: more mispredictions, same stall rule | Scales well |
| Host leaves / app backgrounded | Session stalls. Needs a state snapshot to recover | Same | Needs host migration (a state snapshot again) |
| Precedent | RTS; Nuclear Throne Together (4p co-op mod) | Fighting games; Photon Quantum (mobile, relay server); two-top (Rust, bevy_ggrs, phones) | Source/CS, Overwatch (plus prediction/rollback of the local player), lightyear |

## Per-model notes

### Deterministic lockstep
- Each peer advances only once it has every player's input for that tick. It needs "exactly the same result" from the same inputs, and a playout (jitter) buffer that adds latency ([Gaffer: Deterministic Lockstep](https://gafferongames.com/post/deterministic_lockstep/)).
- Scaling with players is bad: roughly 35% of 4-player matches see significant input delay, because the game waits for the slowest peer ([SnapNet Part 1](https://www.snapnet.dev/blog/netcode-architectures-part-1-lockstep/)).
- **Closest genre precedent: Nuclear Throne Together**, an online co-op mod for 2-4 players.
  - It made the game "behave completely deterministically" and sends only player actions.
  - It hides the delay by handling "critical things (camera, aiming)" without delay.
  - It recommends at most ~100 ms delay / 200 ms ping.
  - PC Gamer reported freezes during heavy action.
  - Sources: [yal.cc intro](https://yal.cc/introducing-nuclear-throne-together/), [NTT FAQ](https://yal.cc/r/17/ntt/), [PC Gamer](https://www.pcgamer.com/nuclear-throne-online-co-op-mod-is-just-as-fast-and-deadly-as-playing-offline/).
- Verdict: works for co-op, but it is fragile on cellular jitter. Worth keeping as a fallback only, and it needs no extra sim work beyond rollback's requirements.

### Rollback (GGPO-style)
- **Requirements:** a fully deterministic frame advance; the ability to "load, save, and execute a single simulation frame without rendering"; the RNG seed identical at frame 0 and the RNG state inside the game state; no time-of-day ([GGPO Developer Guide](https://github.com/pond3r/ggpo/blob/master/doc/DeveloperGuide.md)).
- **GGRS 0.13.0** is the Rust GGPO reimplementation and is engine-agnostic ([docs.rs](https://docs.rs/ggrs/latest/ggrs/), [SessionBuilder](https://docs.rs/ggrs/latest/ggrs/struct.SessionBuilder.html)). What the game provides and what the session is configured with:
  - Save / Load / Advance requests.
  - `Input: Copy + Clone + PartialEq + Default + Serialize + Deserialize`.
  - `with_max_prediction_window` (default 8). Setting it to 0 gives lockstep.
  - `with_input_delay` (default 0).
  - `with_num_players` (default 2).
  - Checksum-based desync detection.
  - `SyncTestSession`, which checks determinism locally without a network.
- **bevy_ggrs 0.22** couples this to Bevy. Our headless crate would use `ggrs` directly.
- **CPU budget is the binding constraint.** At 60 fps with 3 frames of input delay and 300 ms of supported latency, the sim must re-simulate 15 frames in 16.66 ms, which is ~1.1 ms per tick. Their options for heavier sims: lower tick rate, a smaller rollback window (more input delay), or a different architecture ([SnapNet Part 2](https://www.snapnet.dev/blog/netcode-architectures-part-2-rollback/)).
- **Co-op tolerance helps here.** 2-3 ticks of input delay are acceptable in co-op (NTT ran ~100 ms), and they shrink the rollback depth and how visible mispredictions are. Enemies and bullets are driven by deterministic AI, so a misprediction comes only from remote players' inputs.
- **Mobile precedent:** Photon Quantum is a deterministic predict/rollback engine for Unity.
  - It replaces all floats with a 64-bit fixed-point `FP` type (16 fractional bits) and uses lookup-table trig.
  - Float-to-FP conversion inside the sim "will cause desyncs 100% of the time".
  - Systems must be stateless, and events must not drive gameplay. The server relays inputs and runs no game logic, so clients never wait on the slowest peer.
  - Sources: [Quantum intro](https://doc.photonengine.com/quantum/v3/quantum-intro), [Fixed Point](https://doc.photonengine.com/quantum/current/manual/quantum-ecs/fixed-point), [Systems](https://doc.photonengine.com/quantum/current/manual/quantum-ecs/systems). These pages were behind a bot wall; see "Could not verify".
- **Rust precedents:**
  - [two-top](https://github.com/ampactor-labs/two-top) (bevy_ggrs + Matchbox WebRTC, phones) uses Q16.16 fixed-point and bans `f32`/`f64`/`glam` from the sim crate. Its CI compares per-frame checksums across x86_64 Linux, aarch64 Linux, aarch64 macOS and aarch64 Android, plus Wasm.
  - [Vermeilsoft](https://blog.vermeilsoft.com/2025-09-rust-game-engine/) chose fixed-point too (the `fixed` crate). It got a 30-40% faster snapshot clone from `clone_from` reusing allocations.

### Host/server-authoritative snapshot interpolation
- **Needs no determinism.** Clients "don't run any simulation" for remote objects ([Gaffer: Snapshot Interpolation](https://gafferongames.com/post/snapshot_interpolation/)).
- **Costs:**
  - Interpolation delay is ~3x the send interval (~85 ms at 60 Hz, ~300 ms at 10 Hz).
  - Bandwidth scales with object count: 900 cubes is ~25 KB per snapshot.
  - SnapNet adds MTU fragmentation and host CPU that "grows with the square of the number of players" ([SnapNet Part 3](https://www.snapnet.dev/blog/netcode-architectures-part-3-snapshot-interpolation/)).
- **Bullets:**
  - Predicted spawns need deterministic matching, e.g. lightyear's `PreSpawned` hashes archetype + spawn tick ([lightyear book: prespawning](https://cbournhonesque.github.io/lightyear/book/concepts/advanced_replication/prespawning.html)).
  - Enemy bullets are interpolated in the past while the local player is predicted in the present, which is bad for a dodge-focused game.
  - The co-op workaround is to let each client decide hits on its own player. Cheating doesn't matter in co-op.
- **Mobile:** the host is a phone. Its cellular uplink carries every snapshot, and its backgrounding or death kills the session unless host migration exists.
- **Tooling:** [lightyear 0.30](https://github.com/cBournhonesque/lightyear) supports this model, plus input-only "deterministic replication" for lockstep/rollback. It is tied to Bevy 0.19.
- **Overwatch** is server-authoritative with client prediction and rollback of the local player, and it still relies on a fixed tick and deterministic ECS systems for that prediction ([GDC 2017](https://www.gdcvault.com/play/1024001/-Overwatch-Gameplay-Architecture-and)).

### Float determinism, specifically
- **Rust's guarantee** ([RFC 3514](https://rust-lang.github.io/rfcs/3514-float-semantics.html)):
  - `+ - * / % sqrt mul_add abs copysign`, casts and comparisons follow IEEE 754-2008 exactly, *except* for NaN bit patterns, which are non-deterministic.
  - 32-bit ARM NEON flushes to zero.
  - Transcendentals (`sin`, `cos`, `atan2`, `powf`) are **not covered**. They go to the platform libm.
- **So f32 can be made to match on aarch64 iOS/Android and x86_64,** but only with discipline:
  - use the `libm` crate for all transcendentals;
  - no dependency that picks different SIMD or approximation paths per target;
  - never observe NaN;
  - every dependency audited.
- **Rapier** shows the same shape of requirement: cross-platform determinism needs its `enhanced-determinism` feature, IEEE-compliant targets, and nalgebra's math traits instead of std ([Rapier determinism](https://rapier.rs/docs/user_guides/rust/determinism/)).
- **Other sources on why fixed-point is safer:**
  - Gaffer: floats are deterministic with "the same instruction set and compiler", and transcendentals differ across CPUs ([Floating Point Determinism](https://gafferongames.com/post/floating_point_determinism/)).
  - Software FMA fallbacks in compiler-builtins had rounding bugs on targets without hardware FMA ([Shnatsel](https://shnatsel.github.io/implementing-fma-finding-bugs-in-std/)). Hazards like this are why every shipped rollback example above chose fixed-point.
- **Fixed-point costs:**
  - You must watch overflow.
  - `fixed` 1.31 `FixedI32` has `sqrt` and saturating/wrapping ops but **no sin/cos/atan2** ([docs.rs](https://docs.rs/fixed/latest/fixed/struct.FixedI32.html)). You need a LUT or CORDIC.
  - No float physics engine. That is fine for a 2D twin-stick with circle/AABB/tile collision.

## Recommendation

Build the single-player sim as if it were already a GGRS peer: `advance(state, [Input; MAX_PLAYERS]) -> events`, with save/load/checksum.

- Single-player simply runs it locally with zero delay.
- Co-op later means choosing a transport and topology, not rewriting the sim.
- A relay server (Quantum-style) sidesteps cellular NAT and "wait for slowest peer". P2P via Matchbox/WebRTC is the other option.
- If the resim budget fails on device, the same deterministic sim still supports:
  1. more input delay / a smaller prediction window;
  2. lockstep;
  3. host-authoritative snapshots, because state is already serializable.

Choosing snapshot-interpolation-only now would let us skip determinism. But it would close the other two doors for good, and it is the worst of the three for bullet-dodging feel.

## Day-one sim constraints checklist

Cost to add now vs. cost to retrofit: **C** = cheap now, **E** = expensive to retrofit (touches everything), **M** = moderate.

1. **[E] Fixed-point for all sim math.**
   - Use a single numeric type alias (e.g. `type Fx = fixed::types::I32F32` or I16F16; decide via overflow analysis of world size × velocity).
   - Trig via LUT/CORDIC over an integer angle type, e.g. `u16` = one full turn.
   - Ban `f32`/`f64`/`glam` in the sim crate and enforce it with clippy `disallowed_types` or a CI grep.
   - Floats exist only in render/input adapters.
   - Convert float to fixed only at asset-bake time, never at runtime (Quantum rule).
2. **[E] Headless, pure step function.**
   - `fn step(&mut SimState, &TickInputs) -> TickEvents` (or equivalent).
   - No wall-clock, no `dt` argument, no I/O, no globals/`static mut`, no thread-local state.
   - All time is counted in ticks.
   - The tick rate is one constant, and speeds and timers derive from it (see Deferred).
3. **[E] All sim state in one owned value.**
   - `SimState: Clone` (use `clone_from` to reuse allocations), plus `Serialize`/`Deserialize` and a `checksum()`.
   - No `Rc`/`Arc`/pointers/trait objects hiding state, and no caches outside the struct that affect results.
   - This includes the RNG, the entity allocator, timers, the floor/run state machine and the level layout (or its seed).
4. **[E] Deterministic iteration order everywhere.**
   - No std `HashMap`/`HashSet` whose iteration affects results: `RandomState` differs per process. Use `Vec`/slot arenas, `BTreeMap`, or maps with a fixed hasher, and never iterate them for logic.
   - Sort before any order-sensitive reduction.
   - No parallel iteration whose result order matters.
5. **[E] Deterministic, stable entity IDs.**
   - The sim allocates generational IDs (index + generation) from a counter inside `SimState`.
   - Spawn order is deterministic, and an ID is never derived from an address or a render-side handle.
   - The same inputs give the same IDs on every peer and after every rollback. Render maps sim IDs to its own objects.
6. **[E] N player slots from day one.**
   - `MAX_PLAYERS = 4`, with inputs as `[Option<PlayerInput>; 4]` or equivalent. Single-player is slot 0.
   - No "the player" singleton. Enemy targeting, pickups, revives, camera-dependent logic and win/lose all handle N players, with ties broken by slot index.
   - The camera is render-only. Offscreen-culling of *logic* must not depend on any one camera.
7. **[C] Input = small, quantized POD per player per tick.**
   - `#[derive(Copy, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]`.
   - Movement and aim are quantized to integers (e.g. aim as `u16` angle or `i8` x/y), plus button bits.
   - Touch/UI is converted to this *before* it reaches the sim, even in single-player, so what we test is exactly what goes over the wire.
   - Default = "no input" (needed for prediction/disconnect).
   - Menu/shop/upgrade choices that affect the sim are also inputs, applied on a tick. There are no out-of-band mutations.
8. **[C] Seeded RNG inside state.**
   - Use a small, explicitly specified PRNG with no `thread_rng`/OS entropy (e.g. PCG or xoshiro with a pinned algorithm, not `StdRng`, whose algorithm may change between versions).
   - Seed it from the session seed.
   - Consider separate named streams (level gen, loot, combat) so adding a roll in one system doesn't reshuffle others.
   - Cosmetic randomness (particles, shake) uses a render-side RNG and never touches the sim.
9. **[C] Sim → presentation is one-way.**
   - The sim emits per-tick events (spawn, hit, death, sfx cue). Render/audio never mutate the sim.
   - Events must tolerate rollback: presentation may see a tick re-run. Key effects by (tick, entity id) or play only confirmed-tick events for irreversible things.
   - No gameplay logic in event handlers (Quantum rule).
10. **[C] Pause and app-backgrounding live outside the sim.**
    - Single-player pause means "stop calling step", not a sim state.
    - In co-op, "pause" becomes an input-driven sim state or is not allowed. Don't bake an assumption either way.
11. **[C] Keep state compact and flat for cheap save/restore.**
    - Bullets and enemies go in contiguous `Vec`s of POD structs, not boxed per-entity graphs.
    - Target: snapshot a max-density arena in well under 1 ms on the oldest supported iPhone.
    - Spatial-hash collision, no O(n²).
12. **[C] Determinism tests from day one.**
    - (a) Replay: the same seed + recorded inputs give the same checksum per tick.
    - (b) Rollback-every-tick test (GGRS `SyncTestSession` or a homegrown "save, step, load, step, compare"). This catches state that lives outside `SimState`.
    - (c) CI cross-arch check: x86_64 Linux vs aarch64 macOS/iOS simulator, comparing checksum streams (two-top pattern).
13. **[M] Tick cost benchmark.**
    - A headless bench of `step` at worst-case density on device.
    - Rollback budget ≈ `frame_ms / (max_prediction + 1)` per tick. With 8 frames at 60 Hz that is ~1.8 ms, and SnapNet's worst case is ~1.1 ms.
    - If we can't hit it, drop to a 30 Hz sim with render interpolation, or add input delay.
14. **[M] Versioned session config.**
    - Record the sim version/hash + seed + ruleset in `SimState` or the session header, so peers or replays with a mismatch are rejected rather than desyncing.

## Deferred decisions (not needed for single-player; don't pre-build)

- **Transport and topology:** P2P WebRTC via [Matchbox 0.14](https://github.com/johanhelsing/matchbox) (GGRS feature flag), a relay server, or Game Center / nearby (MultipeerConnectivity) for local play.
- **Netcode library:** `ggrs` directly vs. homegrown. `backroll` (0.6.0, last release 2022) looks unmaintained. `lightyear`/`bevy_ggrs` only if we adopt Bevy.
- **Exact tick rate** (30 vs 60 Hz). Decide after the tick benchmark. Keep it a single constant.
- **Networking parameters:** input delay and prediction window values, possibly adaptive to measured RTT/jitter.
- **Recovery:** late join, reconnect after backgrounding, and desync recovery (resync from a peer's serialized `SimState`, which checklist item 3 already enables).
- **Host migration / disconnect policy:** AI takeover, drop the slot, or end the run.
- **Hybrid authority:** e.g. each client authoritative over its own player's damage. Only relevant if we fall back to snapshots.
- **Fixed-point width** (I16F16 vs I32F32 vs I48F16) once world bounds and max speeds are known.

## Could not verify

- **Photon Quantum docs** returned a bot-check page. The Quantum claims above (64-bit FP with 16 fractional bits, LUT trig, float conversion desyncs, relay server with no game logic, stateless systems) come from Photon doc pages as summarized by the search index, not from a direct read.
- **The Valve Source Multiplayer Networking page** returned 403. I did not cite it.
- **No hard numbers were found for US cellular RTT/jitter/loss.** Opensignal publishes a "Games Experience" metric but not the raw values in reachable pages. The mobile jitter assumptions here are qualitative.
- **Carrier-grade NAT making cellular P2P unreliable** (and so favoring a relay/TURN) is common knowledge but was not verified against a primary source.
- **GGRS hard player limit** is not documented. Its default is 2. 4-player co-op over GGRS on phones has no public precedent I found; two-top is 1v1.
- **Rollback cost at bullet-hell entity counts:** I found no published benchmark. It needs our own device benchmark (checklist item 13).
