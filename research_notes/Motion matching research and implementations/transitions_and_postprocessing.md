# Transitions, Blending, and Pose Post-Processing After the Motion Matching Search

Scope: everything that runs after the pose search picks a frame: how the switch is hidden (inertialization, dead blending, blend stacks), how animation root motion is reconciled with the simulated character, foot locking and IK, playback timing, and procedural layers. Pose search, feature design, and neural methods are out of scope (other researchers cover them). Sources were checked on 2026-10-24. Epic documentation pages are versioned; the Motion Warping page fetched was the UE 5.8 version, while some API facts come from UE 5.1 to 5.4 Python API pages, so property names may have moved.

Note on primary sources that could not be read in full:
- The GDC Vault was in maintenance (all `gdcvault.com/play/...` links redirected to `gdconf.com/gdc-vault-maintenance/`), so talk abstracts come from search snippets.
- Bollo's GDC 2018 slide PDF was fetched and its text extracted locally. The slide equations are typeset as glyphs, so only fragments survived. The quintic below was rebuilt from those fragments plus the stated boundary conditions, then checked numerically (details in section 1).
- Naughty Dog's TLOU2 slides are a 3.8 GB PowerPoint and were not downloaded.

---

## 1. Inertialization and its variants: Bollo (Gears of War 4), Unreal's Inertialization and Dead Blending nodes, Holden's spring and dead-blending work, and when systems blend several candidates instead

### Takeaway
Inertialization cuts to the target animation right away. It then adds a decaying offset (target-minus-source pose and velocity, recorded at the switch) so only one animation is evaluated during the transition. Bollo measured 12 µs against 30 µs for a crossfade (60% cheaper) on Xbox One. There are three main forms: Bollo's overshoot-limited quintic (the one Unreal uses), Holden's exact critically-damped spring decay (no fixed duration, can overshoot, cheapest), and dead blending (extrapolate the source with decaying velocity, then crossfade). Dead blending copes better with very different poses and frequent transitions, but it damps detail. Unreal's motion matching can also use an internal blend stack: a real multi-pose crossfade with a bounded number of active blends.

### Cited Findings

**Bollo, Gears of War 4: the origin, the motivation, and the costs**
- Gears of War 4 removed blended transitions entirely and handled motion transitions as an animation post-process. The game "simply cut[s] immediately to the target animation and let[s] the post-process clean up the results", so the source animation is no longer evaluated during a transition. — [Bollo, SIGGRAPH 2017 Talks](https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf)
- Bollo's ladder of ideas:
  - Saving only the last source pose and interpolating toward the moving target gives pose continuity but a visibly discontinuous velocity.
  - Saving one more source pose to match velocity can overshoot after fast source motion (a weapon swings past its target, then recovers).
  - Gears' method matches velocity and guards against overshoot. — [Bollo 2017](https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf)
- The method: "At the start of a transition, we record the source animation's velocity and the difference between the source and target poses. We construct a quintic polynomial interpolating curve that matches the pose difference and velocity at the beginning of the transition and converges to zero by the end … By controlling the curve's initial acceleration, we are able to enforce an overshoot-free transition." — [Bollo 2017](https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf)
- **Cost numbers (Xbox One):**
  - Evaluating one animation sequence costs about 20 µs and a blend about 10 µs.
  - The inertialization curves are slower to evaluate than a simple cubic blend (12 µs vs 10 µs).
  - Overall, 12 µs for inertialization vs 30 µs for the traditional transition (20 µs source evaluation + 10 µs blend) is 60% less, "even in the simplest case where we are blending from only a single animation sequence."
  - 1D blend spaces carry 150% overhead and 2D blend spaces 300% overhead over one sequence. — [Bollo 2017](https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf)
- The GDC 2018 talk "Inertialization: High-Performance Animation Transitions in 'Gears of War'" covers the math for both vector and quaternion values. — [GDC Vault listing (search snippet)](https://www.gdcvault.com/play/1025331/Inertialization)
- **GDC 2018 slide content** (from [Bollo GDC 2018 slides](https://media.gdcvault.com/gdc2018/presentations/bollo_david_inertialization_high_performance.pdf)):
  - **Motivation.** The biggest cost is evaluating both source and target, which costs much more than the blend. In the worst case "everybody transitions at the same time," which doubles animation cost. The intuition is that real humans don't "blend", but they do have inertia.
  - **Goals.** Respect the original animation, with no changes when not transitioning. Be smooth and momentum-preserving. Stay on-model, with no unnatural poses. No overshoot.
  - **Rejected idea #0, filtering the output pose stream:** hard to tune, adds lag, and deviates from the input even when not transitioning.
  - **Idea #1, blend from pose:** capture the pose difference and ease it out.
  - **Idea #2, match velocity:** remember the source velocity through finite differences and use a quintic polynomial (Flash & Hogan 1985, the minimum-jerk arm-movement model).
  - **Idea #3, limit overshoot:** control the initial acceleration, choosing a0 to give zero jerk at t1.
  - **Idea #4, clamp the transition time:** t1 = min(t1, −5·x0/v0). The slide example has x0 = 1.2, v0 = −20, and t1 = 0.5, which is clamped to 0.3.
- **Quintic as reconstructed.** The slide fragments show "−8 … 20" for a0, "3 … 16 … 30", "6 … 12", "12 … 20", and "min(…, −5 …)". Rebuilt with x(t1) = x′(t1) = x″(t1) = 0:
  - a0 = (−8·v0·t1 − 20·x0) / t1²
  - A = −(a0·t1² + 6·v0·t1 + 12·x0) / (2·t1⁵)
  - B = (3·a0·t1² + 16·v0·t1 + 30·x0) / (2·t1⁴)
  - C = −(3·a0·t1² + 12·v0·t1 + 20·x0) / (2·t1³)
  - x(t) = A·t⁵ + B·t⁴ + C·t³ + (a0/2)·t² + v0·t + x0, for t ∈ [0, t1]
  - A local numeric check confirms that x, x′, and x″ vanish at t1 and that jerk vanishes at t1 with this a0. At t1 = −5·x0/v0 the curve touches zero without crossing it. — [Bollo GDC 2018 slides](https://media.gdcvault.com/gdc2018/presentations/bollo_david_inertialization_high_performance.pdf); quintic form x(t) = At⁵+Bt⁴+Ct³+Dt²+v0·t+x0 confirmed by [banming's UE notes](https://banming.github.io/GameEngine/Unreal/animation/Inertialization.html)
- **Unreal's `CalcInertialFloat(x0, v0, t, t1)`**, as described step by step in community notes:
  1. Return 0 once t ≥ t1.
  2. If x0 < 0, negate x0, v0, and the result.
  3. Clamp a positive v0 (pointing away from zero) to 0 to avoid overshoot.
  4. Zero everything on invalid input.
  5. Shorten t1 when v0 is negative and large, so the curve never goes below zero.
  6. Compute a0 and clamp it to ≥ 0.
  7. Compute A, B, C, and D, then evaluate. — [banming, Unreal Inertialization notes](https://banming.github.io/GameEngine/Unreal/animation/Inertialization.html)
  - Conflict to note: the slides choose a0 for zero jerk at t1, while the UE notes add a clamp to a0 ≥ 0.
- **Vectors.** "Obvious choice: inertialize x, y, z independently" produces "visual artifacts if vx0, vy0, vz0 are too dissimilar (because of transition time clamping)". Instead, decompose the vector into direction and magnitude and inertialize only the magnitude. — [Bollo GDC 2018 slides](https://media.gdcvault.com/gdc2018/presentations/bollo_david_inertialization_high_performance.pdf)
- **Quaternions.** "Similar construction to vectors: decompose quaternion into axis and angle; inertialize the angle." The previous-frame offset is reduced to its twist around the current axis, citing Shoemake 1994, "Fiber Bundle Twist Reduction" (Graphics Gems IV); an angle is recovered with a 2·atan form. — [Bollo GDC 2018 slides](https://media.gdcvault.com/gdc2018/presentations/bollo_david_inertialization_high_performance.pdf)
- **Engine integration in Gears:**
  - An Inertialization node is evaluated after the main animation graph. Its input is a discontinuous pose stream and its output an inertialized pose stream.
  - On request, it computes and stores x0 and v0 for all joints, stores t1, and sets t = 0. Every frame it advances t, evaluates and applies x(t) to all joints, and stores the output pose in the pose history buffer (the history feeds the next v0).
  - "Inertialization" is added as a new blend-curve type that zeroes the regular blend time.
  - "Request Inertialization" is exposed to code.
  - Blending vs inertialization: blending has variable per-frame cost and must manage several sets of state; inertialization has a fixed per-frame cost, keeps one set of state, and is "fire and forget." — [Bollo GDC 2018 slides](https://media.gdcvault.com/gdc2018/presentations/bollo_david_inertialization_high_performance.pdf)
- **Tricks from the talk:**
  - Snap character rotation when switching to sprint, then inertialize away the discontinuity.
  - Filter twitchy inputs to locomotion blend spaces; if the filtered values drift too far from the actual values, snap to the actual values and inertialize.
  - Motion warping becomes fire-and-forget, with only one active warp at a time, which simplifies bookkeeping and replication. — [Bollo GDC 2018 slides](https://media.gdcvault.com/gdc2018/presentations/bollo_david_inertialization_high_performance.pdf)

**Unreal Engine's Inertialization node**
- Unreal describes the node as a high-performance alternative to crossfading that works as a post-process: once active, the source pose is no longer evaluated.
- **Placement and triggers.** Place it anywhere after the request source; placing it nearer Output Pose lets it absorb more blends. State machine transitions (Blend Logic), blend nodes (Transition Type = Inertialization), and linked anim graphs or layers can trigger it.
- **Multiple requests.** "When several arrive together … the minimum requested blend duration is used."
- **Guidance.** Short blends work best. Start transitions while the outgoing animation is still moving. Avoid repeated interruptions.
- **Notifies.** Notifies on the source sequence stop firing once inertial blending starts.
- **Filters.** Filtered curves and bones switch instantly.
- **Order.** Insert the node before an additive is applied, and before IK for final fixups. A request with no Inertialization or Dead Blending node downstream logs a runtime error. — [UE Animation Blueprint Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine)
- Conflict on multiple requests: community notes on the UE implementation say the active duration is the longest across the global and per-bone durations, and that requests are queued (`RequestQueue.AddUnique(FInertializationRequest(Duration, BlendProfile))`) and processed in `Evaluate_AnyThread`. The docs say minimum. This may be per-bone max vs across-request min; it is unresolved. — [banming notes](https://banming.github.io/GameEngine/Unreal/animation/Inertialization.html) vs [UE Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine)
- **Implementation details from the community notes:**
  - Per bone, translation is stored as direction + magnitude + speed, rotation as axis + angle + speed, and scale as axis + magnitude + speed.
  - Speeds come from the two previous output poses (`Prev1`, `Prev2`).
  - Blend profiles give per-bone durations (`InertializationDurationPerBone`).
  - Rotation offsets are applied as `Q * rotation`, and translation and scale are added.
  - A teleport (root world-position jump beyond `GetTeleportDistanceThreshold()`) stops the blend.
  - Interruptions are tracked with an accumulated "deficit." — [banming notes](https://banming.github.io/GameEngine/Unreal/animation/Inertialization.html)
- Request API: `RequestInertialization` takes either an `FInertializationRequest` or a duration plus blend profile. The node has a default blend profile used when a request carries none. — [FAnimNode_Inertialization::RequestInertialization](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Engine/FAnimNode_Inertialization/RequestInertialization) (search snippet)
- Epic advice seen in search excerpts: keep blend duration short, "less than 0.4 seconds is best", and when poses are extremely different, do not use inertialization. The exact page was not confirmed; candidates are [UE Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine) and [UE 4.27 Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/blend-nodes?application_version=4.27).

**Holden's spring-based inertialization (offset decay)**
- Record the offset at transition (positions shown; velocity likewise). Adding the existing offset makes interrupted transitions chain correctly:
  - `off_x = (src_x + off_x) - dst_x;`
  - `off_v = (src_v + off_v) - dst_v;`
- Each frame:
  - `decay_spring_damper_exact(off_x, off_v, halflife, dt);`
  - `out_x = in_x + off_x;`
  - `out_v = in_v + off_v;`
- The decay itself, with `halflife_to_damping(h) = (4·ln2)/(h+eps)`:
  - `y = halflife_to_damping(halflife)/2;`
  - `j1 = v + x*y;`
  - `eydt = fast_negexp(y*dt);`
  - `x = eydt*(x + j1*dt);`
  - `v = eydt*(v - j1*y*dt);`
  — [Holden, "Spring-It-On / Spring Roll Call"](https://theorangeduck.com/page/spring-roll-call)
- Holden's trade-offs against the "original presentation['s]" polynomial: a spring "does not provide a fixed blend time and can easily overshoot," but the exponential decay "tends to look smooth," reaches negligible values quickly, and needs no stored transition time. It is very fast when all bones share the same halflife and dt. He used this method for motion matching in "Learned Motion Matching." — [Holden, Spring Roll Call](https://theorangeduck.com/page/spring-roll-call)
- **Reference implementation (orangeduck/Motion-Matching):**
  - Default `inertialize_blending_halflife = 0.1f` (UI range 0 to 0.3 s).
  - The **root is inertialized in world space**: `inertialize_pose_transition` records the root state and converts destination velocities to world space with `quat_mul_vec3(transition_dst_rotation, ...)`.
  - `inertialize_pose_update` reconstructs the world-space root from the input before updating its inertializer. All other bones are inertialized in local space. — [orangeduck/Motion-Matching controller.cpp](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/controller.cpp)
- Other open-source motion matching systems also use halflife-parameterized inertialization; for example, a Godot library does. — [Remi123/MotionMatching](https://github.com/Remi123/MotionMatching) (search snippet)
- A pitfall seen in practice: a quintic decay with zero end velocity and acceleration but no overshoot limiting "can carry a joint past both poses" on long blends from a fast outgoing pose. — [foundation-engine PR #182](https://github.com/Akilleez-QA/foundation-engine/pull/182) (search snippet, low authority)

**Dead blending (Holden, 2023) and Unreal's Dead Blending node**
- **Idea.** At the transition, store the source's current pose x and velocity v. Extrapolate the source forward and crossfade the extrapolation into the destination. No per-joint target-minus-source offset is needed, so the destination doesn't have to be evaluated with velocities on the transition frame. Velocity can come from finite differences. — [Holden, Dead Blending](https://theorangeduck.com/page/dead-blending)
- **Update while t < blendtime:**
  - `v_ext *= exp(-ln2*dt/halflife)` (the decay variant)
  - `x_ext += v_ext*dt`
  - `alpha = smoothstep(t/blendtime)`
  - `out = lerp(x_ext, x_in, alpha)` (slerp for rotations)
- **Joint extrapolation.** Positions use `p += dt*linear_velocity`. Rotations use `q = exp(dt*angular_velocity) * q`, applied on the left. — [Holden, Dead Blending](https://theorangeduck.com/page/dead-blending)
- **Side switching.** The extrapolated pose can drift past 180° from the input, so a shortest-path slerp would flip sides and pop. The fix stores the current path direction d: if dot(d, e) < 0, negate e; set d = e; output `exp(log(e)*alpha)*q`. Reset d to identity at each transition. — [Holden, Dead Blending](https://theorangeduck.com/page/dead-blending)
- **Joint limits.** Either clamp the extrapolation to the limits, or bounce it off them; bouncing looked better but gave no clear gain in motion matching. — [Holden, Dead Blending](https://theorangeduck.com/page/dead-blending)
- **Trade-offs.**
  - Benefits: simple, needs only the current pose, and the output stays between the extrapolated source and the destination, which should in theory avoid knee pops and floaty motion.
  - Drawbacks: it damps destination detail, can foot-slide or look "mushy" like crossfades, and long linear extrapolation over-rotates even at 0.2 s.
  - A learned extrapolator (2 layers × 128 units, ~300 KB of weights, ~30 µs/frame) allows very long blends, but it was tested on its training set. (Neural, so out of scope beyond noting the cost.) — [Holden, Dead Blending](https://theorangeduck.com/page/dead-blending)
- **Unreal version (UE 5.3, experimental).**
  - It stores the pose at the transition and evaluates an implicit extrapolation function of time since the transition, rather than integrating each frame.
  - Quaternions use **per-axis half-lives**, with `y = LN2f/(halflife+eps)`.
  - Per joint axis, the half-life is `clamp(halflife_scale * (src_dst_diff / max_mag(src_vel, eps)), halflife_min, halflife_max)`, where rotation differences are scaled angle-axis. The demo defaults are halflife_scale 0.3, halflife_min 0.1, and halflife_max 1.0.
  - Axes moving slowly toward the destination extrapolate long; axes moving fast or away stop almost immediately. This "avoids most joint limit violations."
  - It is a drop-in replacement for Inertialization and is better for very different poses or frequent transitions, but it can damp motion more. — [Holden, Dead Blending Node in Unreal Engine](https://theorangeduck.com/page/dead-blending-node-unreal-engine)
- **Unreal's documentation on Dead Blending:**
  - Inertialization records the offset and fades it out; Dead Blending predicts future poses of the outgoing animation.
  - Dead Blending is generally better with large pose differences but can look stiffer.
  - Dead Blending respects transition blend curves, which Inertialization ignores.
  - Settings include blend duration and mode or curve, a blend time multiplier, extrapolation half-lives, maximum translation, rotation, scale, and curve velocities, memory preallocation, debug draw of extrapolations, and bone filters.
  - Still marked experimental. — [UE Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine)
- The UE API describes the decay rate as set automatically from how much the source velocities move toward the target pose. Extrapolation half-life is the "average half-life of decay in seconds." — [AnimNode_DeadBlending, UE 5.3 Python API](https://docs.unrealengine.com/5.3/en-US/PythonAPI/class/AnimNode_DeadBlending.html) (search snippet)
- Epic's Game Animation Sample (GASP) uses Dead Blending, which it calls "a better type of inertialization that supports blend curves," to reset its aim offset without popping. — [UE Game Animation Sample](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)

**When systems blend several candidates instead of inertializing (Unreal's blend stack)**
- Unreal's Motion Matching node applies its blend time either by inertial blending (which needs an Inertialization node after the Motion Matching node) or by an internal blend stack when MaxActiveBlends > 0. MaxActiveBlends = 0 disables the blend stack. — [MotionMatchingSettings, UE 5.2 Python API](https://docs.unrealengine.com/5.2/en-US/PythonAPI/class/MotionMatchingSettings.html) (search snippet)
- **Blend stack settings:**
  - Max Active Blends caps the number of simultaneously blending animations.
  - Store Blended Pose folds animations past that cap into a stored pose. With it off, the stack "will start discarding animations, potentially resulting in animation popping."
  - Max Blend in Time to Override Animation replaces the newest animation if it is still inside this window; otherwise a new blend is pushed.
  - Player Depth Blend Time Multiplier speeds up deeper blends.
  - Blend profiles allow per-region blend durations (for example, upper body), and the Mode curve is Linear, Ease In, Ease Out, or Ease In and Out.
  - "Use Inertial Blend" makes blends inertial. — [UE Motion Matching docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- The BlendStack node has an `inertial_blend_node_tag` that routes its inertial requests to a specific Inertialization or Dead Blending node. — [AnimNode_BlendStack Python API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AnimNode_BlendStack) (search snippet; the page itself returned 404/403 when fetched)
- In GASP, Orientation Warping and Steering live inside the Blend Stack graph that opens from the Motion Matching node. Blend time comes from `Get_MMBlendTime` (current and previous states). `Get_MMInterruptMode` forces a blend into a new database when a core state changes. — [UE GASP](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)

### Inferences
- For a Rust engine, the cheapest and simplest production baseline is Holden's exact spring offset decay. It needs per-joint offset position, velocity, rotation, and angular velocity; one shared halflife (~0.1 s); additive re-recording on interrupt; and the root inertialized in world space. It has no duration bookkeeping, so it is "fire and forget" (Bollo's term) and SIMD-friendly, since every joint shares halflife and dt.
- Bollo's quintic is the better choice where fixed blend durations or guaranteed no-overshoot matter, for example design-specified transition times or per-bone blend profiles. It needs a per-transition t1 and the elapsed time, and it gets the magnitude/angle decomposition that keeps axes synchronized after t1 clamping.
- Dead blending is the most robust option against very different poses and rapid re-searching. That is the regime motion matching produces when it searches every ~0.1 s (Holden's demo default `search_time = 0.1f`), but each transition costs a slerp per joint plus extrapolation.
- A blend stack (several animations evaluated at once) costs more animation decoding. Bollo's numbers suggest roughly a sequence evaluation per extra active entry (~20 µs each on Xbox One). It is justified mainly when output must stay strictly a convex combination of real poses, or when the engine already evaluates multiple players.
- Order of operations implied by Unreal's guidance and Holden's demo: (1) search and transition, (2) inertialize or dead-blend the full-body local pose, (3) root reconciliation and warping, (4) additive layers (applied after inertialization), (5) IK and foot locking last.

### Gaps
- No primary source found that defines a "cubic" inertialization variant. Bollo cites cubic only as the cost of a simple crossfade (10 µs). Holden's ["Inertial Easing"](https://theorangeduck.com/page/inertial-easing) uses a smoothstep-based solver for easing toward a target, not for pose transitions (search snippet only).
- The exact vector and quaternion velocity projection formulas on Bollo's slides could not be read (glyph-encoded). The description above is an interpretation of the surviving fragments.
- Unreal's current Inertialization internals (UE 5.4+), including how it handles a request arriving mid-blend and whether it still uses the quintic, could not be read from Epic's source. The docs pages return 403 for Python API versions.
- No source was found describing For Honor's transition method, specifically whether Ubisoft used crossfades or a blend pool.
- No independent benchmark was found that compares spring, quintic, and dead-blending per-frame costs.

---

## 2. Reconciling animation root motion with the simulated character: adjustment and clamps, Animation Warping (orientation, stride, slope), speed warping, distance matching, Motion Warping, and Naughty Dog (TLOU2)

### Takeaway
The standard pattern runs two objects: a simulated capsule driven by a critically damped spring and the animated root. The animated root is pulled toward the simulation with halflife-damped corrections, capped as a fraction of the character's own speed so corrections hide inside motion, and hard-clamped to a maximum distance or angle. Unreal packages this as the Offset Root Bone node plus Steering, then hides the remaining direction and speed mismatch in the pose: orientation warping twists the legs and spine, and stride warping scales the IK feet and drops the pelvis. Distance matching and Motion Warping handle targeted moves (stops, landings, vaults). No technical detail of TLOU2's adjustments could be verified.

### Cited Findings

**Holden: synchronization, adjustment, velocity-limited adjustment, clamping**
- **Options considered:**
  - Sim→character sync gives direct control but slides and spins.
  - Character→sim sync looks grounded but feels sluggish.
  - Blending trades one constraint for another.
  - Doing nothing and relying on motion matching to steer back is ineffective alone. — [Holden, Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- **Adjustment (damped correction, recomputed from zero each frame):**
  - Position:
    - offset = `simulation_position - character_position`
    - damped by `g * (1 - fast_negexpf((LN2f*dt)/(halflife+eps)))`
    - applied as `adjustment_position + character_position`
  - Rotation:
    - offset = `quat_abs(quat_normalize(quat_mul_inv(simulation_rotation, character_rotation)))`
    - damped as a shortest-path slerp from identity by the same factor
    - applied as `quat_mul(adjustment_rotation, character_rotation)`
  - Rotation is usually damped more slowly, because fast rotational correction looks bad. — [Holden, Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- **Velocity-limited adjustment:**
  - Position cap: `max_length = max_adjustment_ratio * length(character_velocity) * dt`
  - Rotation cap: the same with `character_angular_velocity`, applied on scaled angle-axis.
  - With ratio 0.5, rotation is corrected only while the character already turns, and position mostly when moving fast. This reduces foot sliding. — [Holden, Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- **Clamping:**
  - If the distance exceeds `max_distance`, project the character onto the sphere: `max_distance * normalize(character_position - simulation_position) + simulation_position`.
  - Angles are measured with `2*acosf(clampf(diff.w,-1,1))` and the relative rotation is clamped to ±max_angle.
  - Bounded deviation helps networking and collision, but can feel like being "dragged along." — [Holden, Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- Holden prefers editing the animation data to suit the simulation, for example speeding mocap up by ~10%. His database builder resamples to ~60 fps with a 10% speed-up. — [Holden article](https://theorangeduck.com/page/code-vs-data-driven-displacement); [generate_database.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/generate_database.py)
- **Demo defaults:**
  - `simulation_velocity_halflife = 0.27`, `simulation_rotation_halflife = 0.27`
  - `adjustment_position_halflife = 0.1`, `adjustment_rotation_halflife = 0.2`
  - `adjustment_position_max_ratio = 0.5`, `adjustment_rotation_max_ratio = 0.5`
  - `clamping_max_distance = 0.15` (metres), `clamping_max_angle = 0.5π`
  - Synchronization is off by default. When on, it replaces adjustment and clamping.
  - Per-frame order: inertialize_pose_update → simulation update → (sync | adjust → clamp) → foot locking + IK → forward kinematics. — [controller.cpp](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/controller.cpp)
- The demo's simulation (root) bone is derived from the data: position from Spine2 projected to the ground (Savitzky-Golay window 31), and facing from the projected Hips forward (window 61). — [generate_database.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/generate_database.py)

**Unreal: Offset Root Bone and Steering (the engine-packaged version of the same idea)**
- Offset Root Bone (experimental) has these parameters:
  - TranslationMode / RotationMode
  - TranslationHalflife / RotationHalfLife ("lower values blend faster")
  - MaxTranslationError (units) / MaxRotationError (degrees); negative disables the cap
  - `bClampToTranslationVelocity` / `bClampToRotationVelocity`, which limit blend-out to the incoming motion's speed "to prevent sliding when there is little movement"
  - TranslationSpeedRatio / RotationSpeedRatio
  - Collision testing (mode, shape offset, radius)
  - `bOnGround` + GroundNormal, which project root motion onto the ground
  - `bResetOnTeleport` — [FAnimNode_OffsetRootBone API](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/AnimationWarpingRuntime/FAnimNode_OffsetRootBone)
- **GASP usage of Offset Root Bone:**
  - Translation modes: Release blends the offset out and locks to the capsule; Interpolate lets the root deviate within a radius and pulls it back.
  - Translation is released when stopped, in the air, or in a montage. `Get_OffsetRootTranslationHalfLife` is faster when stopped.
  - Rotation modes: Release locks to the capsule; Accumulate counter-rotates capsule changes so root motion and steering control rotation.
  - Known issues: no collision checks on the translation offset (it can clip geometry), unwanted movement on montage release, and a hard-coded interpolation curve. — [UE GASP](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)
- GASP's documented purpose for Offset Root Bone: it "is intended to fix mesh deviations from the capsule during animation playback … the system will blend the mesh's position back toward the capsule's center when the character stops moving." — [UE GASP (search snippet)](https://dev.epicgames.com/documentation/unreal-engine/game-animation-sample-project-in-unreal-engine?lang=en-US)
- **Steering node.** It "adds procedural delta to the root motion attribute" in two combined ways: it scales the animation's rotational root motion to reach TargetOrientation, and adds a correction after accounting for the anticipated (scaled) root motion. Its properties:
  - AnimatedTargetTime: look-ahead into the clip's root motion.
  - ProceduralTargetTime: seconds for the additive correction to reach the target.
  - MinScaleRatio / MaxScaleRatio: leftover error is compensated linearly.
  - RootMotionThreshold (editor: RootMotionAngleThreshold): minimum root motion before scaling.
  - DisableAdditiveBelowSpeed: the spring-based additive is off below this speed.
  - DisableSteeringBelowSpeed: all steering is off below this speed.
  - Mirror support. — [FAnimNode_Steering API](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/AnimationWarpingRuntime/FAnimNode_Steering)
- GASP blocks steering while idle to prevent sliding, and steers toward the trajectory's predicted future facing (`GetDesiredFacing`) instead of the current actor rotation. — [UE GASP](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)
- Warping nodes have a graph-driven mode, where parameters are "automatically computed using the accumulated root motion delta contribution of the animation graph leading into it", and a manual mode. — [WarpingEvaluationMode, UE Python API](https://docs.unrealengine.com/PythonAPI/class/WarpingEvaluationMode.html) (search snippet)
- Community report: GASP's workflow exports the capsule's simulation and has the animation follow it "really tightly" to limit mocap foot sliding. — [UE forum thread](https://forums.unrealengine.com/t/how-to-perfectly-align-animation-root-motion-with-the-movercomponents-trajectory/2746566) (search snippet; community, unverified)

**Animation Warping (Pose Warping) nodes**
- **Orientation Warping.**
  - It warps the leg IK bones toward the locomotion direction and twists the spine to keep facing. In graph mode the angle is RotationBetween(RootMotionDirection, LocomotionDirection).
  - Location Angle Delta Threshold prevents over-rotation; the default is 90°, and 0 disables it.
  - Spine Bones are ordered so index 0 is the highest and least rotated bone.
  - Distributed Bone Orientation Alpha (1 puts all rotation at index 0, 0 at the highest index) controls how the twist is spread across the spine.
  - Rotation Axis Z is recommended.
  - Rotation Interp Speed is in alpha per second; 0 means instant.
  - "Warping can only work within the animation's coverage." — [UE Pose Warping](https://dev.epicgames.com/documentation/en-us/unreal-engine/pose-warping-in-unreal-engine)
- **Stride Warping.**
  - It scales foot spacing to match capsule speed. Stride Scale = LocomotionSpeed / RootMotionSpeed (default 1); Min Locomotion Speed Threshold gates it.
  - The scale has min/max clamping and separate increasing and decreasing interpolation speeds.
  - A Pelvis IK Foot Solver pulls the pelvis down, with a stiffness/damping-ratio interpolator, max distance, error tolerance, and max iterations.
  - Options: Orient Stride Direction Using Floor Normal, Compensate IK Using FK Thigh Rotation (preserves leg shape), and Clamp IK Using FK Limits (prevents over-extension). — [UE Pose Warping](https://dev.epicgames.com/documentation/en-us/unreal-engine/pose-warping-in-unreal-engine)
  - Stride warp settings: Warp Forwards (<1 shortens, >1 lengthens) and Sideways Offset (negative pulls the IK goals toward the centerline). — [MakeStrideWarpSettings](https://dev.epicgames.com/documentation/unreal-engine/BlueprintAPI/Utilities/Struct/MakeStrideWarpSettings) (search snippet)
- **Slope Warping.**
  - Feet are warped to floor normals for inclines and stairs.
  - Settings: pelvis offset, floor normal, and floor offset interpolators (stiffness and damping ratio), Max Step Height, Keep Mesh Inside Of Capsule, and Pull Pelvis Down.
  - Epic: "This node is still in development, don't trust a project to its functionality." — [UE Pose Warping](https://dev.epicgames.com/documentation/en-us/unreal-engine/pose-warping-in-unreal-engine)
- Lyra uses Orientation Warping plus a Root Yaw Offset to shape turns. — [Epic tech blog, adapting Lyra animation](https://www.unrealengine.com/en-US/tech-blog/adapting-lyra-animation-to-your-ue5-game) (search snippet)
- GASP turn-in-place triggers when root and capsule rotation differ by more than 50° while aiming, and also after a stop ("Stick Flick"). Epic notes it can lag "too far behind without extra limits on steering or root offset." — [UE GASP](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)

**Speed warping and distance matching (Paragon lineage)**
- Laurent Delayen's nucl.ai 2016 talk on bringing a Paragon hero to life in UE4 covered the locomotion system, its layers, and processing. A forum post ties Unreal's distance matching to that Paragon work. — [gameanim.com](https://www.gameanim.com/?p=14556); [UE Answers, Anim Distance Matching Plugin](https://answers.unrealengine.com/questions/769283/view.html) (search snippets)
- **Distance matching basics.**
  - The animation is driven by a distance variable instead of time. A Distance Curve Modifier bakes a curve of root distance; Sample Rate 30 covers most cases.
  - Stop Speed Threshold (default 0) handles pivots that never fully stop. Axis is Z for landings and XY for start, stop, and pivot. "Stop at End" is available.
  - The curve needs the Uniform Indexable compression codec so it can be read at runtime. — [UE Distance Matching](https://dev.epicgames.com/documentation/en-us/unreal-engine/distance-matching-in-unreal-engine)
- **Distance matching functions.**
  - `DistanceMatchToTarget(evaluator, DistanceToTarget, curve)` sets time to where the curve equals the remaining distance (stops, landings).
  - `AdvanceTimeByDistanceMatching(…, DistanceTraveled, curve, PlayRateClamp)` advances by distance travelled (starts).
  - `SetPlayrateToMatchSpeed(player, SpeedToMatch, PlayRateClamp)`.
  - It "effectively changes the play rate … to keep the feet from sliding"; it is "common to clamp the resulting play rate" and use Stride Warping "to make up the difference." — [UAnimDistanceMatchingLibrary API](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/AnimationLocomotionLibraryRuntim-/UAnimDistanceMatchingLibrary)
- Third-party plugin descriptions use "Speed Warping" for stretching leg stride to match speed, which is the predecessor concept to Stride Warping. — [Fab listing](https://www.fab.com/listings/576d3579-7f0a-4412-a5bc-e3970d06ed96?lang=de) (search snippet; third party, low authority)

**Motion Warping (targeted root-motion alignment)**
- **Gears of War 4 warp points.** Animators place markers ("warp points") at physical landmarks in their scenes. At runtime the game places matching warp points in the world and modifies the character's trajectory so the two align, with independent timing control for translation, rotation, and facing. This builds on Gleicher 2001 and Lockwood & Singh 2011. It is "over twice as fast as a more traditional blend space based approach" because it evaluates one sequence. — [Bollo 2017](https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf)
- **Unreal Motion Warping.**
  - It uses Motion Warping notify-state windows in a root-motion montage, a Motion Warping Component, and warp targets set by name ("Add or Update Warp Target").
  - Modifiers listed: Skew Warp (match the target location and rotation by the end of the window) and Scale. A setup guide also uses "Adjustment Blend Warp."
  - Options: Warp Translation, Ignore Z Axis, and Warp Rotation, with Rotation Type Default (match the target's rotation) or Facing (turn to face the target).
  - Warp Rotation Time Multiplier: 0.5 on a 2 s window finishes rotating in 1 s.
  - Warp point providers: None, Static transform, or Bone. — [UE Motion Warping (5.8)](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-warping-in-unreal-engine); [Art of Making, Motion Warping](https://artofmaking.substack.com/p/motion-warping)
  - Skew Warp has an easing setting for adding translation "used when there is no translation in the animation." — [RootMotionModifier_SkewWarp API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/RootMotionModifier_SkewWarp) (search snippet)
- GASP uses montages for traversal. `JustTraversed` selects the tail end of traversal animations when blending back to locomotion, and Motion Matching can also pick montages and entry frames for gameplay actions. — [UE GASP](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)

**Naughty Dog, The Last of Us Part II**
- Talk: "Motion Matching in The Last of Us Part II" (GDC 2021) by Michal Mach (Lead Animator) and Maksym Zhuravlov (Principal Gameplay Animator). The abstract covers only the project arc: early enthusiasm, then frustration, then fear. The slides are a 3.8 GB PPTX. — [Naughty Dog at GDC 2021](https://www.naughtydog.com/blog/naughty_dog_at_gdc_2021)
- Mach's reel: the system was used "mainly for character locomotion but also traversal, melee and player/npc cover system," and allows "pretty reliable foot matching when blending into traversal animations." — [Mach reel on Vimeo](https://vimeo.com/433024043) (search snippet)
- Co-director Anthony Newman (2019) described clips chopped into small pieces and matched to the character's path, with as little blending as possible at each foot plant and turn. It was used for NPCs, horses, and dogs too. — [GamingBolt](https://gamingbolt.com/the-last-of-us-part-2s-new-motion-mapping-technique-provides-incredible-next-gen-leap-in-animations); [Wccftech](https://wccftech.com/the-last-of-us-part-ii-motion-matching-tech/) (press, secondary)

### Inferences
- Byte-Engine can implement Holden's adjust → clamp pair as two pure functions over (character transform, simulation transform, character velocity, dt, params) → new character transform. That matches the project's preference for pure functions and separate layers. The same pair, run in reverse, gives Unreal's "Interpolate/Accumulate" offset-root behaviour.
- Velocity-proportional caps (Holden's max_ratio, Unreal's ClampToVelocity/SpeedRatio) are the key anti-slide ingredient: corrections are only allowed while the animation itself moves or turns. Idle needs a separate policy: Unreal's "Release when stopped" with a faster halflife, or disabling steering below a speed.
- Pose-space warps (orientation and stride) are the second line of defence. They change the pose so the remaining mismatch doesn't show as sliding. Both need leg IK targets ("IK foot bones") and a pelvis solver, so they belong after the root-reconciliation stage and before final foot IK.
- Distance matching and Motion Warping solve a different problem from continuous reconciliation: hitting a known future target (a stop point, a ledge). In a motion matching system they are typically used for montage-like actions or for the selected clip's remaining segment, not for every frame.

### Gaps
- No technical detail of TLOU2's root adjustment, steering, IK, or post-processing could be verified. The slides were not downloaded (3.8 GB) and no detailed secondary summary was found.
- The Skew Warp and Adjustment Blend Warp algorithms (how the remaining root-motion deltas are redistributed) are not documented publicly; the math lives in Epic's engine source.
- Default values for Orientation Warping Rotation Interp Speed, Distributed Bone Orientation Alpha, the Offset Root Bone halflives, and Steering times were not found.
- The Delayen Paragon talk (speed warping, orientation warping origins) could not be accessed; only its abstract is known.
- Sean Dickinson, "Motion Warping in 'Gears of War 4': Doing More with Less" (GDC 2017) is cited in Bollo's slides but was not accessed.
- No Bobby Anguelov / Esoterica material on root motion, warping, or transitions surfaced in searches.

---

## 3. Foot locking and IK: Holden's inertialized foot lock, two-bone IK, slopes and stairs, pelvis adjustment, contact detection and labels, ground alignment

### Takeaway
Holden's lock does three things:
- It labels toe contacts offline: toe speed under 0.15 m/s, majority-filtered over 6 frames.
- At runtime it locks the contact point and feeds it through its own inertializer, so locking and unlocking blend smoothly. The lock releases on contact end or once the point is more than 0.2 m from the animated position.
- It solves a closed-form two-joint IK.

Unreal's Foot Placement and Stride/Slope warping add speed-based plant detection, a pelvis drop and heel-lift policy, and floor-normal alignment. Holden calls foot locking a last resort: small slides often look better than distorted poses.

### Cited Findings
- **Contact labels in Holden's database.**
  - Only `LeftToe` and `RightToe` are labeled. Contact = toe global speed < `contact_velocity_threshold = 0.15` (m/s; data in metres at 60 fps). There is no height threshold.
  - Each track then goes through `ndimage.median_filter(size=6, mode='nearest')` as a "majority vote" to remove short runs.
  - Contacts are computed after mirroring, so mirrored clips get their own labels.
  - The result is stored per frame as `contact_states` (uint8, two columns). — [generate_database.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/generate_database.py)
- **Runtime foot lock.**
  - When contact begins, the lock point is taken from the foot's ground-projected position with `contact_point.y = foot_height`.
  - A `contact_lock` flag and an inertializer feed the locked point (with zero velocity) instead of the animated one.
  - The lock releases when contact ends or when `length(contact_point - input_contact_position) > unlock_radius`.
  - The toe lock is used to compute a heel location, and a two-joint IK pins the foot.
  - Extras: preventing ground penetration and bending the toe on contact.
  - "Foot locking [is] a last resort, since small slides often look better than a distorted pose." — [Holden, Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- **Demo defaults:** `ik_max_length_buffer = 0.015`, `ik_foot_height = 0.02`, `ik_toe_length = 0.15`, `ik_unlock_radius = 0.2`, and `ik_blending_halflife = 0.1`. Contact bones are the toes, and contact states come from the database (`db.contact_states(frame_index)`), not from runtime detection. — [controller.cpp](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/controller.cpp)
- **Two-joint IK** (hip a, knee b, heel c, target t):
  1. Clamp the hip-to-target distance to `[eps, lab + lcb - eps]` so the limb never locks straight or fully folds.
  2. Get the current interior angles from dot products and the desired angles from the law of cosines; the difference gives the hip and knee bends.
  3. Swing the hip so the heel lands on the target, rotating around `cross(c-a, t-a)`.
  - The bend axis `cross(c-a, b-a)` becomes unstable near full extension. Use a reference pole vector d (the knee's forward axis from its global rotation) and the axis `cross(c-a, d)`.
  - World-space axes are moved to local space by the inverse global rotation, then rotations are post-multiplied onto the local rotations.
  - It is closed-form and constant-time. — [Holden, Simple Two Joint IK](https://theorangeduck.com/page/simple-two-joint)
- **Unreal Foot Placement node (Animation Warping plugin).**
  - Plant detection: `speed_threshold` (planted below this speed, read from an FK speed curve) and `unalignment_speed_threshold` (fully unplanted). The range between them "should roughly represent the roll-phase of the foot", which Epic flags as often inaccurate.
  - Other plant settings: `distance_to_ground`, `replant_radius_ratio`, and `replant_angle_ratio`.
  - `plant_speed_mode` is Graph (root motion attributes) or Manual (per-foot curve).
  - Pelvis: `heel_lift_ratio` (1 = lift the heel before dropping the hips, 0 = drop the pelvis first), `max_offset` (vertical cap; feet may then miss their plant plane), and `max_offset_horizontal`.
  - `pelvis_height_mode` (All Legs, All Planted Feet, …); enable pelvis interpolation to smooth swaps of the "planted" leg. — [FootPlacementPlantSettings](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/FootPlacementPlantSettings); [FootPlacementPelvisSettings](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/FootPlacementPelvisSettings); [AnimNode_FootPlacement](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AnimNode_FootPlacement) (search snippets)
- **Pelvis and slopes in the warping nodes.** Stride Warping's Pelvis IK Foot Solver pulls the pelvis down (spring interpolation, max distance, iterations). Slope Warping aligns feet to floor normals, with Max Step Height and Pull Pelvis Down, but is not production-ready. — [UE Pose Warping](https://dev.epicgames.com/documentation/en-us/unreal-engine/pose-warping-in-unreal-engine)
- **GASP leg IK.** Simple Leg IK pins the feet to the IK foot bone locations, which pose warping often moves. Epic says it "can improve blending when using blend profiles on the legs." — [UE GASP](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)
- Unreal says to apply inertialization before IK for final fixups, so IK targets are computed on the already-smoothed pose. — [UE Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine)

### Inferences
- Byte-Engine pipeline split, consistent with the project's layer-separation rule:
  1. An offline contact labeler: pure, per clip, writing a per-frame bitset.
  2. A runtime contact-lock state machine (pure: label, animated point, previous lock state → lock state and target).
  3. An inertializer reused from section 1 to smooth lock targets.
  4. An IK solver (pure two-joint).
  5. A pelvis policy (drop to the lowest reachable foot target, clamped).
- Speed-threshold contacts (Holden: one threshold plus a median filter; Unreal: two thresholds framing the roll phase) are cheap and robust. A height threshold is optional; Holden's data uses none.
- Locking the toe and deriving the heel (Holden), or locking both ball and heel (Unreal's "Foot/Ball speed"), keeps the toe roll intact. Locking only the ankle tends to freeze the foot flat.

### Gaps
- No source found for ground-normal alignment math (foot rotation onto a slope) beyond Unreal's slope-warping settings list. Holden's two-joint IK article doesn't cover foot orientation.
- The stair-specific techniques (step-height prediction, foot trace scheduling) used in shipped motion matching games were not found in primary sources.
- Defaults for Unreal Foot Placement thresholds were not found.
- [BodeSoftware/FootContactCurves](https://github.com/BodeSoftware/FootContactCurves) appeared in a search (a tool for generating foot contact curves) but was not examined.

---

## 4. Timing and pace: playback-rate scaling, time warping, and keeping feet in phase across transitions

### Takeaway
Playback rate is a bounded corrective. Unreal's Motion Matching node takes a play-rate interval meant to absorb movement-model vs animation speed mismatches; its result reports a "wanted" rate (query trajectory speed ÷ result speed). Distance matching clamps its implied rate and hands the rest to stride warping. Foot phase across transitions comes from motion matching itself (it searches on foot features) or, in state-machine systems, from Bollo's phase-quotient lookup.

### Cited Findings
- Unreal's `AnimNode_MotionMatching.play_rate` is a FloatInterval: "effective range of play rate that can be applied to the animations to account for discrepancies in estimated velocity between the movement model and the animation." An experimental `play_rate_multiplier` scales the selected clip's rate after the motion matching update. — [AnimNode_MotionMatching Python API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AnimNode_MotionMatching) (search snippet)
- `WantedPlayRate` on the pose-search result is "usually calculated by estimating the ratio between the trajectory speed in the query against the speed in the result." — [PoseSearchBlueprintResult Python API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/PoseSearchBlueprintResult) (search snippet)
- **Unreal's motion matching anti-repeat and timing knobs** (these act on what is played next):
  - Pose Jump Threshold Time avoids "picking sections of animation inside the same clip over and over".
  - Pose Reselect History.
  - Search Throttle Time.
  - Continuing Pose Cost Bias and Looping Cost Bias.
  - Notify filtering (Notify Recency Time Out, default 0.2 s, "a baseline for biped locomotion"), so notifies don't double-fire across rapid transitions. — [UE Motion Matching docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- **Rate correction elsewhere in Unreal.** Distance matching changes the play rate to stop sliding, is clamped (`PlayRateClamp`), and stride warping makes up the difference. `SetPlayrateToMatchSpeed` assumes constant speed. — [UAnimDistanceMatchingLibrary](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/AnimationLocomotionLibraryRuntim-/UAnimDistanceMatchingLibrary); [UE Distance Matching](https://dev.epicgames.com/documentation/en-us/unreal-engine/distance-matching-in-unreal-engine)
- Stride warping is the pose-space alternative to rate scaling: Stride Scale = LocomotionSpeed / RootMotionSpeed, clamped and interpolated. — [UE Pose Warping](https://dev.epicgames.com/documentation/en-us/unreal-engine/pose-warping-in-unreal-engine)
- Holden adjusts the data's pace offline (+10% speed) to fit the simulation, instead of scaling playback at runtime. — [Holden, Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement); [generate_database.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/generate_database.py)
- **Bollo's phase matching (Gears of War 4).**
  1. Compute a scalar "phase quotient" from the left foot's position relative to the right, projected on velocity (or on forward when stationary). It oscillates roughly sinusoidally.
  2. Negate and offset it during the incoming half-cycle so it increases monotonically over the whole cycle.
  3. At startup, tabulate it per frame for the main cycles, filter it to be monotonic, invert it, and fit a piecewise-linear curve from phase back to frame.
  4. When re-entering locomotion, compute the current pose's quotient and start the cycle at the mapped frame. — [Bollo 2017](https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf)
- Holden's demo forces a new search when the desired velocity or rotation change crosses a threshold (50.0) and otherwise searches every 0.1 s or at the end of a clip. This sets how often transitions, and so inertializations, happen. — [controller.cpp](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/controller.cpp)
- Mach's TLOU2 reel credits motion matching with "pretty reliable foot matching when blending into traversal animations." — [Mach reel](https://vimeo.com/433024043) (search snippet)

### Inferences
- For Byte-Engine, rate scaling should be a clamped scalar (for example a [0.8, 1.2]-style interval; the specific range is a design choice, not sourced) applied before root reconciliation. Leftover speed mismatch should go to stride warping, and from there to root adjustment.
- Bollo's phase quotient is cheap and useful outside motion matching: re-entering locomotion from montages, traversal, or hit reactions without a search.

### Gaps
- No primary source was found for "time warping" in the strict sense (dynamic time warping or variable-rate retiming) applied at runtime after a motion matching search in shipped games.
- No published default for Unreal's motion matching play-rate interval was found.
- Sync groups / sync markers in Unreal (marker-based phase sync for blend spaces) were not researched in this pass.

---

## 5. Procedural layers on motion matching: look-at, leaning, additives, upper-body overlays and layered blending, and physics-based hit reactions or partial ragdoll

### Takeaway
Shipped setups keep motion matching as the full-body (or lower-body) base and add four kinds of layer:
- additive leans driven by acceleration
- aim offsets in strafe or aim modes
- per-bone layered blends for upper-body actions
- physics layers whose weight ramps from the hit bone down the chain

Order matters. Inertialize before applying additives, and run IK last. Specific sources on combining motion matching with physics hit reactions were not found; the available material is generic Unreal physical-animation guidance.

### Cited Findings
- **GASP layers:**
  - Additive lean comes from a BlendSpace driven by relative acceleration, normalized to −1..1 and scaled by speed.
  - The aim offset engages in strafe mode when the character faces the camera within a threshold, and is disabled during default-slot montages.
  - Dead Blending resets the aim offset without popping.
  - The Default Slot injects traversal montages. — [UE GASP](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)
- **Layered Blend Per Bone:**
  - Branch Filter (bone + Blend Depth) or Blend Mask. Positive depth tapers the weight along the chain, 0 applies full weight to the bone and its children, and negative favours the base pose.
  - Mesh Space Rotation and Scale Blend options.
  - Curve blend options (Override, Blend by Weight, Use Base Pose, …).
  - "Blend Root Motion Based on Root Bone." — [UE Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine)
- **Additives.** Apply Additive / Apply Mesh Space Additive add a scaled additive pose onto a base. Make Dynamic Additive builds one by subtraction. Inertialization should come before the additive is applied. — [UE Blend Nodes](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine)
- **Upper vs lower timing.** The motion matching node's blend profile "determines how blending is distributed among the character's bones" and "could be used to differentiate between upper body and lower body to blend timing." — [AnimNode_MotionMatching Python API](https://docs.unrealengine.com/5.3/en-US/PythonAPI/class/AnimNode_MotionMatching.html) (search snippet); [UE Motion Matching docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- **Gears practices that apply to layered systems:** snap rotation discontinuities and inertialize them away; filter twitchy inputs to blend spaces, snapping and inertializing when the filtered value strays too far. — [Bollo GDC 2018 slides](https://media.gdcvault.com/gdc2018/presentations/bollo_david_inertialization_high_performance.pdf)
- **Unreal hit reactions with partial ragdoll:**
  1. Find the hit bone.
  2. Call `SetAllBodiesBelowSimulatePhysics` from that bone (its children simulate too; a clavicle gives a limp arm).
  3. Ramp `SetAllBodiesBelowPhysicsBlendWeight` toward 1.0 and back to 0.0, setting it every tick.
  4. Disable simulation once the weight is back at 0.
  - At 1.0 the bones are fully physics-driven; at 0.0 they return to keyframes. — [UE Physics-Driven Animation](https://dev.epicgames.com/documentation/en-us/unreal-engine/physics-driven-animation-in-unreal-engine)
- **Unreal Physics Control plugin.** The RigidBodyWithControl anim node simulates the physics asset inside the anim graph. Control targets "may be defined as an offset from the (implicit) animation target," expressed relative to the parent body. Layering on skeletal animation (`use_skeletal_animation`) is documented as Physics Control Component–only. — [RigidBodyControlTarget](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/RigidBodyControlTarget); [PhysicsControl plugin API](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Plugins/PhysicsControl) (search snippets)
- **Pose-matching ragdolls (Unity, open source).** Forces track the target animation's world-space position and per-bone local-space rotation, so collisions and impulses can still deform the pose. — [Hairibar.Ragdoll](https://github.com/hairibar/Hairibar.Ragdoll) (search snippet)
- **Holden's demo has no layers.** Its order is inertialize, simulate, adjust, clamp, IK, FK, with no additive or upper-body layer. — [controller.cpp](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/controller.cpp)

### Inferences
- A layered Byte-Engine graph could follow this sequence:
  1. Motion matching base pose.
  2. Inertialize or dead-blend.
  3. Root reconciliation.
  4. Orientation and stride warp.
  5. Upper-body layered blend, with the upper-body overlay transitions using their own inertializer instance so they don't reset lower-body offsets.
  6. Additives (lean, aim).
  7. Look-at as a final rotation layer on the spine, neck, and head.
  8. Foot lock and IK.
  9. Physics layer (blend weight per hit chain).
- Physics hit reactions are best treated as a final-pose consumer. The physics target is the fully post-processed kinematic pose, and the ramp back to 0 must finish before the next search makes big pose changes (or the physics output gets inertialized back into the stream).

### Gaps
- No primary source was found on look-at systems layered on motion matching (spine distribution, clamps, eye/head/neck weighting) in shipped titles.
- No source was found that covers motion matching combined with physics-based hit reactions or active ragdoll in a shipped game, including how the motion matching search is treated during or after a physics reaction (for example searching from the physically perturbed pose).
- Unreal's Physical Animation Component (strength profiles, local vs world-space motors) was not documented on the page fetched.
