# GUI engine hiccups from the third_person HUD

**Building the TLOU2-style HUD in `../sandbox/third_person` hit one silent layout bug, four defaults and no-ops that fail quietly, and several gaps that pushed per-app plumbing into the sample.** The worst problem was that a second element at a component's root never draws, and the first one ignores its position. Three other things also fail without a word: the default white fill, an edge feather that does nothing on an element's own fill, and a component's lack of access to the viewport size. The rest is friction. There are no right or bottom anchors, a fade can't be retargeted with `animate`, feeding application state takes a polling loop, and every application copies the same host boilerplate.

The HUD is in `third_person/src/hud.rs`. Each section below gives the symptom, the cause, the workaround the sample uses, and a direction for the fix. The sections are ordered by how much time each cost and how likely it is to bite the next person.

## 1. Only the first root element is laid out, at (0, 0)

**Symptom.** The HUD cluster was declared with `root.element("cluster").container(|c| c.position((724.0, 884.0)) …)`. It drew at the top-left corner. The aim circle, a second element declared at the root, never appeared. Nothing was logged. Logging `cluster.geometry()` showed `0 0 252 92` against a target of `(724, 884)`.

**Cause.** `layout/mod.rs` picks one root with `tree.parents.iter().position(Option::is_none)`. That is the first parentless element. It then calls `place(tree, root, …, Offset::new(0.0, 0.0), false, …)`, which ignores the root's own `Position`. Other parentless elements are never placed, ranked, or drawn.

**Workaround.** The sample declares one full-window `screen` container and hangs everything from it.

**Fix direction.** Treat every parentless element as a root. Place each one from the viewport and honour an absolute `Position`. At minimum, log an error naming the dropped element. `docs/use/gui.mdx` shows a single element at the root, so its examples don't reveal the rule either way. There's a proposed session card for this one.

## 2. A container paints a white box unless told otherwise

**Symptom.** A container used only for placement, such as a row of pips, a positioned text box or the ring holder, fills its whole box with white.

**Cause.** `ConcreteStyle::default()` holds one `ConcreteLayer::default()`, which is a white fill (`style.rs`). Only `ConcreteStyle::new()` is empty. I found this by reading the isometric sample's `label` helper, which passes `ConcreteStyle::new()` without explaining why.

**Workaround.** A `layout_only` helper in `hud.rs` sets `.clip(false).hit_testable(false).style(ConcreteStyle::new())` on every container that only places its children.

**Fix direction.** Make an unstyled container invisible. That means painting nothing until a style is set, which matches what `ConcreteStyle::new()` documents as the empty case.

## 3. Overlay containers need three opt-outs each

**Symptom.** Every HUD element needed `hit_testable(false)`, and most also needed `clip(false)`, because a HUD must neither take the pointer nor cut off its children.

**Cause.** `Container::default()` sets `clip: true` and `hit_testable: true`.

**Workaround.** The same `layout_only` helper, plus `hit_testable(false)` on every painted element.

**Fix direction.** Choose defaults that cost nothing, with clipping and hits off until asked for. Alternatively, let a subtree opt out of hit testing once, at its root.

## 4. A layer's edge feather does nothing to its own fill

**Symptom.** `ConcreteLayer::feather_edges(0, 0, 0, w)` on the weapon panel's fill left a hard left edge.

**Cause.** In `layout/engine/clipping.rs`, `inherited_visual_state` reads the first layer's feather only when the container clips, and then only as a mask on its descendants. The layer's own fill never uses it. `ConcreteLayer::feather` has no doc comment saying so.

**Workaround.** A horizontal `LinearGradient` that goes from transparent to 55% black.

**Fix direction.** Either feather the layer's own fill, which is what the setter's name and position suggest, or move the setting to the container as a descendant mask and document it there.

## 5. Components can't anchor right or bottom, or read the viewport

**Symptom.** Putting the cluster 48 px from the bottom-right corner, and the aim circle at the centre, meant computing pixel positions from the window size.

**Cause.** Flow functions such as `row`, `column`, `grid`, `center` and the centred variants place children from the top left or the centre. There's no right, bottom or corner anchor. A component also can't ask the engine for the viewport size it is being evaluated at.

**Workaround.**
- The application context (`Hud`) carries a `viewport: Size`.
- `Game::show_hud` listens to `ghi::window::Events::Resize` and updates that size.
- The component compares it with the previous frame's value and repositions the cluster and the aim circle.

**Fix direction.**
- Add anchored positions: an offset from any edge or corner of the parent, or a flow function that pins a child to a corner.
- Expose the evaluated size to components, for example through `ctx.viewport()`.

With both in place, a HUD no longer needs the size in its application state or a resize branch of its own.

## 6. The host owns window size and repeats the same boilerplate

**Symptom.** Each application writes the same steps by hand:
1. Create a `Factory<Render>`.
2. Call `setup_ui_render_pass` at the right point in the pass order.
3. Build `Engine::with_context`, then `mount`.
4. Listen for resize events.
5. Call `evaluate(size, app.frame_allocator())`.
6. Compare `render().revision()` with the last one published, and call `create(render.clone())` when it changed.

I copied this from the isometric sample's `Interface::update`. The UI sandbox has its own copy, including a different resize-to-layout mapping (`viewport_size`).

**Cause.** The engine leaves the evaluate-and-publish step and the window size to the application.

**Fix direction.** Have the engine own that step and track the window's size itself, in line with "the engine owns state" and slim interfaces. The application would only mount components and set its context.

## 7. A fade that must retarget can't use `animate`

**Symptom.** The cluster fades in on activity and fades out after 3 idle seconds. Activity during a fade-out must turn it around at once.

**Cause.** `animate` holds the component until its driver settles, so its loop can't change the target midway. `render()` resolves to `()` and gives no frame delta.

**Workaround.** A per-frame loop measures `Instant` deltas, steps two `Smoothing` drivers itself, and writes each value with `update_container(|c| c.opacity(..))` until they settle (`step_opacity` in `hud.rs`).

**Fix direction.** Do one of these:
- Let `render()` return the frame's `MediaTime` delta.
- Offer a retargetable property animation, such as a smoothed opacity on an element whose target a component sets while the engine steps it.

## 8. Feeding application state takes a polling loop

**Symptom.** The HUD reads `ctx.with(|hud| *hud)` every frame and compares the result with the last snapshot to decide what to edit.

**Cause.** Nothing wakes a component when its application context changes. `ui::Watch` does wake its subscribers and may be the intended path. The isometric sample uses `Cell`s and polling instead, so the samples don't show a convention.

**Workaround.** Poll every frame. This is cheap because only changed values cause edits, but the component still runs once per frame forever.

**Fix direction.** Document the intended way for game state to reach a component, `Watch` or otherwise, and use it in the samples. If `Watch` is the answer, the HUD's per-frame loop only needs to stay alive while a fade is running.

## 9. Text can't be positioned by itself

**Symptom.** Placing the "9mm pistol" label took a wrapping container.

**Cause.** `Properties<Text>` has `content` and `font_size`, but no `position` or `absolute_position`. Only containers carry a position.

**Workaround.** A `weapon` container holds the text.

**Fix direction.** Give text, and the other non-container primitives, a position setter. Otherwise, name the wrapper pattern in the guide.

## 10. Smaller friction

- **No centred row with a gap.** `row_with_gap` and `column_with_gap` exist, but `centered_row` has no gap variant. The ammo divider is a 12 px spacer container with a 1 px line centred inside it.
- **No baseline alignment.** The large loaded count and the small reserve count in one `centered_row` line up by box centre, not by text baseline. It looks acceptable at these sizes but would show at larger size differences.
- **Colour space after tone mapping is unclear.** The isometric sample converts sRGB to linear because "the UI pass blends linear values". It then runs `setup_srgb_display_render_pass`. The third_person sample runs AgX tone mapping, which already writes display-encoded output, and then the UI pass. I wrote the HUD's colours as display values and they look right. Neither doc says which space UI colours are in when the UI pass follows a tone mapper.
- **No UI view in the inspector.** The inspector serves screenshots, entities and messages, but not the UI tree or its geometry. Finding problem 1 took a temporary `eprintln!` of `geometry()`. A route that dumps each element's id, key path and laid-out rectangle would have shown it at once.

## What worked well

- **`Sector` made the segmented ring trivial.** Each segment is a container with `Sector::new(start, step, 0.9).inset(gap / 2)`, and the constant-width gaps came out right on the first try.
- **`LinearGradient` and `opacity` on a subtree** worked as expected, and so did the `update_text` and `update_container` edits.
- **Edits are cheap.** The HUD only changes styles, text and opacity, so layout never replays while it follows the game.
