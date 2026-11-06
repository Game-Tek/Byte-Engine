# Make-native plan: last 90 days (2026-08-08 → 2026-11-06)

682 non-merge commits. I skipped bug fixes, as the skill says. I also skipped the reshapes that already happened: the "Simplify … (second pass)" commits, "Streamline …", and "shrink visibility impl". The candidates below come from the feature diffs and from what is in `dev` today. I verified each one exists before listing it here. Order: highest payoff and lowest risk first.

## 1. GI techniques as one choice, not four booleans
- Source: 34fcc38b5 (probe GI), e8d8c09f2 (surfel GI), d64591831 (lightmaps), 54f721ec8 (SSGI and contact shadows left out of the bake).
- Smell: `VisibilityFeatures` (`rendering/pipelines/visibility/features.rs:90`) carries `ssgi`, `surfel_gi`, `probe_gi`, and `baked_gi` as separate bools. On top of those sit `*_ENABLED_PARAMETER` constants and a `gi.technique` selector that has to agree with them. The `indirect_diffuse` umbrella rule exists only to re-derive "any GI is built in". Each technique added one more bool, one more parameter, and one more branch.
- Native shape: a single built-in GI set, for example a small bitset keyed by the `gi.technique` names that `gi.technique` already parses. That keeps it an open set of names, not a new enum. `indirect_diffuse` becomes `!set.is_empty()`. The four `*_ENABLED_PARAMETER` constants collapse into one `gi.techniques` list parameter.
- Also: `built_in_or_off` (`manager.rs:695`) is applied by hand to each feature. Make it the one place where runtime settings meet the built-in set, driven by the set.
- Tests: rewrite the feature-parsing tests to the list parameter. Delete the tests that check each bool separately.
- Docs: update the reference spec first (`docs/reference`, the render parameters), then the code.

## 2. Unattended mode as a pacing mode, not a flag threaded through the renderer
- Source: ba568d549.
- Smell: the application holds `unattended: Option<Duration>` (`application/graphics/mod.rs:98`). The renderer holds a second `unattended: bool` (`renderer/core.rs:111`) set through `set_unattended`, and `if !self.unattended` checks are scattered through window visibility and the present mode. The pacing memory says the renderer only reports state, and pacing lives in the application loop.
- Native shape: the application loop owns a `Pacing` value, either attended (`max-frame-rate`, render-on-demand) or fixed period. It passes the renderer what it needs per frame (which windows to render, and the present mode), not a mode flag. Delete `set_unattended`.

## 3. Alley `execute_with_caller` / `tick_with_lanes` as the only execute path
- Source: 1e5c9aefd.
- Smell: a `_with_` variant next to the original. Check whether plain `execute` is `execute_with_caller` with a no-op caller. If it is, keep one function. This touches a hot path: run the alley benches before and after, using the interleaved A/B method.

## 4. Pixel-step fusion: `record_with_constant` and `fusion_enabled`
- Source: 2c57f21eb.
- Smell: `simple_tile.rs:233 record_with_constant` sits beside `record`, and a `fusion_enabled: bool` gates fusion. An unfused pass is a fusion run of length 1. If `.fusion` assets are the mechanism, the unfused path should be that degenerate case, not a separate one. Keep a constant-less `record` only if a caller really has no constant.

## 5. Material `dynamic: bool` (four copies)
- Source: 53003a320.
- Smell: `resources/material.rs:196,205,679,688` repeats `dynamic` across the description and baked types, plus a `stale` bool in the renderer. Check whether "dynamic" can come from the BEMX graph, where a parameter node is either constant or exposed (104cfa9d4 made every material a graph). If it can, the flag duplicates what the graph already encodes.

## 6. Inspector address and port
- Source: 9da2bac35 (port), 368245dcf (address), c7cfa255d (streamline).
- Smell: two parameters added one after the other. Merge them into one `inspector.address` socket address with a loopback default. Small change.

## 7. Debug views threaded with `#[cfg(feature = "headed")]`
- Source: 5916c2a87.
- Check: `debug_views` is also a `VisibilityFeatures` bool, alongside the `debug_view_` member-name convention. Fold it into the same built-in set as item 1 if that falls out naturally, and remove the scattered `cfg` attributes by putting the pool behind the existing headed boundary.

## Looked at, leave alone
- HID hotplug (c4538bb72 → 9d5ac17a0): already reshaped, with hidapi deleted.
- BELD roots, exclude, and prune (64053f535, 6258cc8e4, c46952a31): already streamlined in 1360ffea0.
- Animation routes, one-shots, and round-robin: each one extends the graph's own state and route model. Nothing bolted on is visible from the subjects alone, but item 8 below checks this.
- Physics, BETP, and the message bus: these were redesigns, not bolt-ons.

## 8. Second-tier audit (not yet read in depth)
Before doing these, read the diffs of: the animation graph additions (7372df51a, fc1fdeed4, e54d7c72f, 27a2795f8), motion matching (6cdafa887, 77e79db74), bake intercepts (4fbd59089: `read`/`write` bools), and de36919cb ("bake one generated container material on its own").

## How to run it
One commit per item, `refactor:` style subjects in the repo's sentence-case convention. Each commit runs `cargo nextest run --workspace`, `cargo test --doc --workspace`, and the benches when it touches a hot path (items 1, 3, and 4 touch rendering and the alley). Cross-check Vulkan/DX12 compiles for 1 and 4. Items 1, 2, and 5 change the public API or parameters, so sketch them to you before implementing.
