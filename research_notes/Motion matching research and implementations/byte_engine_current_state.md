# Byte-Engine's current animation, character, and locomotion stack, and where motion matching would fit

Sources are local files, surveyed read-only on 2026-10-24 at commit `0f1601df3` (branch `dev`). Paths that start with `crates/`, `docs/`, or `CHANGELOG.md` are under `/Users/facundovilla/development/Byte-Engine`; paths that start with `third_person/` or `isometric/` are under `/Users/facundovilla/development/sandbox`. A `:N` suffix is a line number. Nothing was built, tested, or benchmarked; every timing below comes from the repository's own docs or changelog.

## 1. Where does animation live, what are its core types, and how are clips imported, baked, and stored?

### Takeaway
All skeletal animation lives in `crates/byte-engine/src/animation` (about 9,900 lines). It is a CPU pipeline: FBX or glTF goes to an rkyv-archived `Animation` resource of uncompressed f32 curves. A load worker packs each clip into an uncompressed word layout in a pooled arena. A code-built state machine with 1D and 2D blend spaces samples at most three clips per motion, blends them, inertializes transitions, extracts root motion, applies a look chain, and posts affine global matrices for GPU skinning. Nothing quantizes or compresses keys. The engine has no database, feature, or search concept anywhere.

### Cited Findings
**Module layout**
- `animation/mod.rs` declares `blend`, `flipbook`, `graph`, `inertialization`, `math` (crate-private), `packed`, `root_motion`, `skeletal`, `system` — [crates/byte-engine/src/animation/mod.rs:10-20](crates/byte-engine/src/animation/mod.rs)
- File sizes: `graph.rs` 1,632 lines; `graph/runtime/player.rs` 3,785; `graph/runtime.rs` 523; `graph/motion.rs` 454; `graph/runtime/player/look.rs` 267; `graph/runtime/player/stream.rs` 542; `graph/runtime/benchmarks.rs` 419; `packed.rs` 557; `blend.rs` 520; `inertialization.rs` 422; `system.rs` 565; `skeletal.rs` 385; `root_motion.rs` 84; `math.rs` 155 (from `wc -l`).

**Resource types (crate `resource-management`)**
- `LocalTransform` holds `translation: Vector<ParentSpace>`, `rotation: Orientation`, `scale: Scale` — [crates/resource-management/src/resources/skeleton.rs:9-13](crates/resource-management/src/resources/skeleton.rs)
- `SkeletonNode { name: Option<String>, parent: Option<u32>, rest_local }`, and `Skeleton { nodes }` in parent-before-child order — [skeleton.rs:34-37, 143-144](crates/resource-management/src/resources/skeleton.rs). Validation enforces that order "for allocation-free hierarchy evaluation" — [skeleton.rs:308-309](crates/resource-management/src/resources/skeleton.rs)
- `SkeletonPoseMap::by_name` retargets between rigs by node name; target-only nodes keep rest — [skeleton.rs:148-160](crates/resource-management/src/resources/skeleton.rs)
- `SkinBinding::write_matrix_palette` writes final skin matrices without allocating — [skeleton.rs:58-72](crates/resource-management/src/resources/skeleton.rs)
- `Curve<V,T>` is `Step`, `Linear`, or `CubicSpline` (with tangents). Typed as `TranslationCurve`, `ScaleCurve`, and `RotationCurve` (whose tangents are raw quaternion derivatives) — [crates/resource-management/src/resources/animation.rs:16-41](crates/resource-management/src/resources/animation.rs)
- `NodeTrack { node: u32, translation, rotation, scale }` (all optional) and `Animation { name, skeleton: Reference<Skeleton>, duration: f32, frame_rate: Option<f32>, tracks }`. `frame_rate` is `None` for glTF — [animation.rs:190-206](crates/resource-management/src/resources/animation.rs)
- Clip curves are stored "as resource metadata rather than binary payload bytes" — [animation.rs:210-218](crates/resource-management/src/resources/animation.rs)
- Resources serialize with rkyv 0.8 (`rkyv::to_bytes` / `rkyv::from_bytes`) — [crates/resource-management/src/lib.rs:103-130](crates/resource-management/src/lib.rs), [crates/resource-management/Cargo.toml:42](crates/resource-management/Cargo.toml)

**Import (bake)**
- FBX goes through `ufbx::bake_anim(... trim_start_time: true ...)`. Every baked node track becomes a `Curve::Linear` with no further reduction, and tracks are sorted into dense node order. `frame_rate` comes from `scene.settings.frames_per_second` — [crates/resource-management/src/asset/handler/implementations/fbx/skeleton.rs:107-166, 223-244](crates/resource-management/src/asset/handler/implementations/fbx/skeleton.rs)
- glTF keeps the sampler's own interpolation (STEP, LINEAR, CUBICSPLINE), normalizes rotations, keeps tangents as authored, and validates key times — [crates/resource-management/src/asset/handler/implementations/gltf/skeleton.rs:482-640](crates/resource-management/src/asset/handler/implementations/gltf/skeleton.rs)
- Fragments select clips: `#animation`, `#animations/0`, `#animations/Walk`, plus `#skeleton`. A container with one clip and no mesh bakes as `Animation`. A `.bead` JSON5 sidecar can set `default_resource: 'animation'` and, for FBX, `skeleton` (bake against another file's skeleton). glTF morph-weight channels are rejected — [docs/reference/assets.mdx:144-149, 188-243](docs/reference/assets.mdx)

**Packing (load time, not bake time)**
- `AnimationLoadWorker::run` requests the `Animation` resource, then calls `PackedAnimationData::from_resource` on the worker "to keep the per-key work off the thread that calls `AnimationPool::update`" — [crates/byte-engine/src/animation/graph/runtime.rs:385-409](crates/byte-engine/src/animation/graph/runtime.rs)
- Packed layout: a 6-word header; a 7-word track table; 4-word timelines. Curves keyed at identical times share one timeline. Values are stored key by key, so a pose sample reads two adjacent rows. Up to 16 timelines go on the stack (`STACK_TIMELINES`) — [crates/byte-engine/src/animation/packed.rs:1-34](crates/byte-engine/src/animation/packed.rs)
- Values are stored as f32 bit patterns in `u32` words (`f32::from_bits`/`to_bits`). Rotations are normalized ahead of time, so no quantization or compression happens — [packed.rs:76-81, 164, 193, 303, 385-388](crates/byte-engine/src/animation/packed.rs)
- `PackedAnimation::sample_pose` and `sample_nodes` (the latter samples only a few nodes, "the root motion node and its ancestors") are `pub(crate)`, and so are `PackedAnimationData` and `TrackTargets` — [packed.rs:40, 85-117, 225](crates/byte-engine/src/animation/packed.rs)
- The developer note: "Clips baked from FBX key every curve at the same frames, so they have one timeline" — [docs/develop/animation.mdx:60-62](docs/develop/animation.mdx)

**Pool**
- `AnimationPool` preallocates one `Box<[u32]>` arena sized by a byte budget and tracks free ranges with `utils::RangeAllocator`. It keys clips by resource ID in a fast hash map and evicts the least recently sampled clip — [runtime.rs:83-99](crates/byte-engine/src/animation/graph/runtime.rs); [CHANGELOG.md:268](CHANGELOG.md)
- The system's default budget is 32 MiB (`pool-bytes` parameter) — [crates/byte-engine/src/animation/system.rs:160-176](crates/byte-engine/src/animation/system.rs). Load queue capacity is 64 — [graph.rs:50-53](crates/byte-engine/src/animation/graph.rs)

**Sampling and global pose**
- `sample_local_pose` fills a caller-owned `Vec<LocalTransform>`; missing curves keep rest — [crates/byte-engine/src/animation/skeletal.rs:16-35](crates/byte-engine/src/animation/skeletal.rs)
- `write_global_pose` multiplies parent × local `AffineMatrix` in node order — [skeletal.rs:42-63](crates/byte-engine/src/animation/skeletal.rs)
- Shared curve math: `KeySpan::find` (binary search with `partition_point`), lerp/nlerp, and Hermite with tangents scaled by the span — [crates/byte-engine/src/animation/math.rs:9-155](crates/byte-engine/src/animation/math.rs)

**Graph (state machine)**
- The module doc covers code-authored state machines and async clip pooling. A state plays one clip, a round robin, or a linear (1D) or planar (2D) blend. States can share a blend position through spaces (`planar_space`, `linear_space`). Routes are `.when`/`.anytime`, optionally through a one-shot clip or blend — [crates/byte-engine/src/animation/graph.rs:1-47](crates/byte-engine/src/animation/graph.rs)
- `StateTransition` fields: `matching_frames`, `target`, `destination`, `duration` ("critically damped inertialization duration"), `can_interrupt_transition`, `wait: Box<[ClipPoint]>`, and `entry: Box<[ClipPoint]>` — [graph.rs:115-131](crates/byte-engine/src/animation/graph.rs)
- Route modifiers: `matching_frames()`, `at_frame`, `at_frames(clip, frames)`, `at_time`, `entering_at_frame`, `with(motion)`, `exit(d)`, `turning(yaw)`, and `any_state_to` — [graph.rs:740-890, 987](crates/byte-engine/src/animation/graph.rs)
- Clients select states "with normal Rust control flow" and the player reconciles toward the requested state — [graph.rs:6-8](crates/byte-engine/src/animation/graph.rs)

**Player, system, and output**
- `AnimationGraphPlayer` keeps the graph ID, root motion and look settings, runtime state, the latest request, the missing-clip list, and an optional debug stream — [crates/byte-engine/src/animation/graph/runtime/player.rs:411-428](crates/byte-engine/src/animation/graph/runtime/player.rs)
- `AnimationGraphPose { skeleton, local_pose, global_pose: &[AffineMatrix], root_motion: RootMotionDelta, state }` — [player.rs:391-403](crates/byte-engine/src/animation/graph/runtime/player.rs)
- Per-frame stages, in order: routes, residency, sampling, blending, root motion, visual pose, global pose — [docs/develop/animation.mdx:39-47](docs/develop/animation.mdx)
- `AnimationSystem::step`: adopt streams, retire deleted animators, adopt created ones, apply requests, `pool.update()`, `advance_players` on the alley, then for each player send `UpdatePose` (headed builds) and `RootMotionUpdate` — [crates/byte-engine/src/animation/system.rs:213-240](crates/byte-engine/src/animation/system.rs)
- `UpdatePose` carries one `AffineMatrix` per node, copied with `to_vec()` at the system boundary, which the doc calls "currently intentional" — [crates/byte-engine/src/rendering/pose.rs:2-22](crates/byte-engine/src/rendering/pose.rs); [graph.rs:44-46](crates/byte-engine/src/animation/graph.rs)
- GPU skinning deforms bind-pose vertices with matrix or dual-quaternion palettes into the resident vertex streams — [crates/byte-engine/src/rendering/pipelines/visibility/skinning.rs:1-31](crates/byte-engine/src/rendering/pipelines/visibility/skinning.rs). The visibility scene exposes `write_skinned_pose` — [crates/byte-engine/src/rendering/pipelines/visibility/scene.rs:207](crates/byte-engine/src/rendering/pipelines/visibility/scene.rs)
- Separate from skeletal animation, `Flipbook` plays image sequences (used by the isometric sandbox) — [crates/byte-engine/src/animation/flipbook.rs:1-17](crates/byte-engine/src/animation/flipbook.rs)

### Inferences
- The end-to-end path is: FBX/glTF → `AnimationModel` (rkyv, f32 curves, metadata) → `ResourceManager::request::<Animation>` → load worker packs → pool arena → `PackedAnimation::sample_pose` into canonical-skeleton local pose → `blend_local_poses` → root-motion delta from the root chain → `PoseInertializer` → look chain → `write_global_pose` → `UpdatePose` → GPU skinning.
- Packing happens at load, not at bake, so no offline step exists yet where per-frame features could be computed. A motion matching database would be the first animation artifact that needs real bake-time processing (many clips in, one matrix out). It could also be the first animation resource whose bulk data lives in a binary payload instead of rkyv metadata.
- The packed sampler and `TrackTargets` are `pub(crate)`, so a motion matching implementation inside `byte-engine` can reuse them directly; one outside the crate could not.
- Every key is a full f32 with no compression. Memory: one 64-node clip at 30 fps with all three channels on every node is about 64 × 10 floats × 30 ≈ 19,200 floats (≈ 77 KB) per second. This is an estimate, not a measurement.

### Gaps
- I did not find which resample rate `ufbx::bake_anim` uses with default `BakeOpts`, so the exact key density of FBX clips is unknown.
- I did not inspect the actual sizes of the MotusMan clips. The sandbox resources are stored under hashed names in `sandbox/resources`.

## 2. How does locomotion work today: navigation, gait selection, character control, physics, and input?

### Takeaway
Locomotion is hand-built in application code (the `third_person` sandbox), not in the engine. Gameplay turns input or navigation into an "intent" (movement direction relative to the body, sprint, crouch, facing). It eases direction, speed, and facing with exponential responses, picks a graph state, and sends a planar-blend position (angle × speed level). Root motion from the clips then moves the transform directly. Navigation only answers paths and steered velocities, and the NPC picks its gait from the distance along the path. The engine has no character controller: characters are static capsules that root motion moves, and physics runs after them.

### Cited Findings
**Sandbox character driver (`third_person`)**
- The walk blend is one planar state: a centre idle, an 8-clip walk ring at radius 1, and an 8-clip sprint ring at `SPRINT_SPEED_LEVEL = 2.0`, triangulated explicitly (a fan to the walk ring plus a band to the run ring) — [third_person/src/animation.rs:22-23, 72-111](/Users/facundovilla/development/sandbox/third_person/src/animation.rs)
- The graph uses `standing` and `crouched` planar spaces; start blends and stop blends follow the space position; turns in place, jumps (with landing "twins"), and deaths (a round robin plus `any_state_to`) — [third_person/src/animation.rs:61-340](/Users/facundovilla/development/sandbox/third_person/src/animation.rs)
- Clip catalog sizes: 12 starts, 12 stops, 16 turns, 15 jumps, 8 walk, 8 sprint, 8 crouch-walk, 4 deaths. 104 distinct `anims/…fbx` resource IDs appear in the file (counted with `grep -o | sort -u`) — [third_person/src/animation.rs:545, 568, 694, 757, 892, 904, 924, 936](/Users/facundovilla/development/sandbox/third_person/src/animation.rs)
- Stop fit frames are hand-measured tables. "The frames were measured by comparing every stop's first pose with every frame of its loop … Sprint stops wait at most 0.2 s and start within 9-37 cm of the run's pose instead of up to 63 cm; walking and crouched stops … within 2-19 cm instead of up to 23 cm" — [third_person/src/animation.rs:560-567](/Users/facundovilla/development/sandbox/third_person/src/animation.rs)
- `CharacterIntent { movement: Option<[f32;2]>, sprinting, crouching, jump, facing_yaw, look }` is the controller-independent input shared by the player and NPCs — [third_person/src/animation/character.rs:24-37](/Users/facundovilla/development/sandbox/third_person/src/animation/character.rs)
- `Motion::update` eases the blend angle (`DIRECTION_RESPONSE = 8`), the speed level (`SPEED_RESPONSE = 3`), and a turnaround past `TURNAROUND_ANGLE = 135°`, which jumps the blend and requests `inertialized(150 ms)`. It keeps a stop grace (`STOP_DELAY = 0.1 s`) and a direction memory (0.3 s) — [character.rs:166-226, 395-409](/Users/facundovilla/development/sandbox/third_person/src/animation/character.rs)
- `select_state` turns the body toward the desired facing while moving (`FACING_RESPONSE = 8`). Standing, it turns in place past `TURN_IN_PLACE_THRESHOLD = 65°`. It then requests `evaluate_animation_state(locomotion, …, position=[sin, cos](heading), speed_level)` — [character.rs:249-311, 360-374](/Users/facundovilla/development/sandbox/third_person/src/animation/character.rs)
- Root motion settings: node `Hips`, translation X|Z, rotation `None`. `apply_root_motion` rotates the delta by the character's orientation, adds it to the position, and composes the rotation (which carries only hand-over turns) — [character.rs:339-351, 377-381](/Users/facundovilla/development/sandbox/third_person/src/animation/character.rs)
- The driver comment says it "retains only the motion shaping and animation rules that the graph cannot yet express" — [character.rs:1-5](/Users/facundovilla/development/sandbox/third_person/src/animation/character.rs)

**NPCs and navigation**
- NPCs replan every `PLAN_PERIOD = 0.1 s`. They pick a `Gait {Stand, Walk, Sprint}` from the path length alone: start beyond 4.5 m, stop within 3.5 m, sprint when `length − speed × 0.6 s > 5 m`. They aim 0.8 m along the path (`AIM_AHEAD`) and call `navigation.steer(npc, toward * gait.speed())` — [third_person/src/npc.rs:179-240, 361-389, 423-466](/Users/facundovilla/development/sandbox/third_person/src/npc.rs)
- The heading turns at `turn_rate(speed) = min(LATERAL_ACCELERATION (10 m/s²) / speed, 150°/s)`. `WALK_SPEED = 1.4`, `SPRINT_SPEED = 6.5` m/s — [npc.rs:342-345, 427-443](/Users/facundovilla/development/sandbox/third_person/src/npc.rs)
- NPCs then hand a `CharacterIntent` (direction relative to the body, sprint flag) to the same `CharacterAnimation` driver, so their actual displacement comes from root motion, not from the steered velocity — [npc.rs:258-310](/Users/facundovilla/development/sandbox/third_person/src/npc.rs); [npc.rs:5-6](/Users/facundovilla/development/sandbox/third_person/src/npc.rs)
- The engine's Use page advises: "Pick the gait from the distance to the target alone, not from the path's turns", turn "at a rate bounded by how hard it can lean into a turn, such as 10 m/s² divided by its speed, up to 150° per second", and aim about 0.8 m ahead — [docs/use/navigation.mdx:96-108](docs/use/navigation.mdx); commit `301487648`
- Engine navigation API: `Navigation::update(delta)`, `find_path(agent, target, &mut path)`, `steer(agent, velocity) -> Vector`, and `NavigationAgent::new(radius).with_precedence(p)` — [crates/byte-engine/src/gameplay/pathfinding/navigation.rs:58-78, 206, 297, 333](crates/byte-engine/src/gameplay/pathfinding/navigation.rs)
- Agent velocity is estimated from transform displacement, averaged over about 0.25 s (`VELOCITY_AVERAGE_SECONDS`) — [navigation.rs:433](crates/byte-engine/src/gameplay/pathfinding/navigation.rs); [docs/reference/navigation.mdx:64-74](docs/reference/navigation.mdx)
- Steering is one continuous rule set. Against each crowd of equal or greater precedence it sidesteps the shortfall at the closest approach (2 s horizon, 0.15 m comfort, at least 0.25 s). Against the boundary it slides so the agent reaches it in no less than 0.5 s, and off the mesh the boundary is one-way — [docs/reference/navigation.mdx:76-117](docs/reference/navigation.mdx); [crates/byte-engine/src/gameplay/pathfinding/steering.rs:1, 151-154](crates/byte-engine/src/gameplay/pathfinding/steering.rs)
- Path detours bend the first leg around a crowd within 4 m — [docs/reference/navigation.mdx:119-135](docs/reference/navigation.mdx); commit `dd79a5686`
- Commit `00c45fae3` added "a response time [that] lets agents that can't change course at once, such as root-motion characters, plan from where they will be". The next commit, `e7e6b05d8`, removed it: "Trajectory checks and response times are gone with the plans they served" — `git show` of those commits.

**Character control and physics**
- I found no character-controller type. A grep for `CharacterController|KinematicCharacter` returned nothing. `BodyTypes` are `Static`, `Kinematic`, and `Dynamic` — [crates/byte-engine/src/physics/body.rs:126-133](crates/byte-engine/src/physics/body.rs)
- "Kinematic | Takes the same path as static today: no mass, no gravity, never moved by the solver." A non-rotating body option exists "for characters" — [docs/reference/physics.mdx:15-31](docs/reference/physics.mdx); [CHANGELOG.md:170](CHANGELOG.md)
- Ray queries are answered after the next step; "There is no shape cast, overlap, or sweep query" — [docs/reference/physics.mdx:116-130](docs/reference/physics.mdx)
- The sandbox player and NPCs are `BodyTypes::Static` capsules used as shot targets — [third_person/src/player.rs:46-53](/Users/facundovilla/development/sandbox/third_person/src/player.rs); [third_person/src/npc.rs:57-58](/Users/facundovilla/development/sandbox/third_person/src/npc.rs)
- Tick order: controls → `navigation.update` → task poll (intent and requests) → `animation.step` → task poll (apply root motion, camera, fire) → `physics.step` — [third_person/src/game.rs:95-116](/Users/facundovilla/development/sandbox/third_person/src/game.rs); same order prescribed in [docs/reference/animation.mdx:308-324](docs/reference/animation.mdx) and [docs/use/animation.mdx:81-127](docs/use/animation.mdx)

**Input**
- Actions bind triggers to typed values. WASD map to `Axis3` directions that resolve to one normalized vector. The `WhileActive` tick policy emits every frame while a value is held — [docs/reference/input.mdx:55-112](docs/reference/input.mdx); [third_person/src/controls.rs:94-130](/Users/facundovilla/development/sandbox/third_person/src/controls.rs)
- The inspector's `TriggerAction` message posts an action event as if from a device (headless input) — [docs/reference/inspector.mdx:46-60](docs/reference/inspector.mdx)

### Inferences
- The sandbox's `Motion` easing (exponential responses on direction, speed, and facing) already works as an implicit desired-trajectory model. A motion matching trajectory predictor (for example a critically damped spring on velocity and facing) would replace or formalize it.
- The engine is fully root-motion driven with no simulation-side controller, so motion matching would first be "animation drives the entity". A Holden-style simulation-bound variant (controller drives, animation follows, then warping or adjustment) has nothing to bind to until a kinematic controller exists.
- Navigation already produces what a trajectory query needs for NPCs: a path polyline (`point_along_xz`), a steered velocity, and a bounded turn rate. Steering no longer models root-motion lag (the response time was removed), so a motion matching system would have to absorb the difference between steered velocity and achieved root-motion velocity.
- The hand-measured stop tables are a manual, offline pose search. They show the exact problem motion matching automates: choosing the clip and frame that fits the current pose and velocity.

### Gaps
- Some hand-tuned constants have no source data on how they were derived (for example `TURNAROUND_ANGLE`, `SPEED_RESPONSE`); only their doc comments explain intent.
- I did not find how jumps interact with vertical motion (root motion keeps only X|Z), and I found no ground-following code.

## 3. Which blending and transition methods exist, and is there IK, foot locking, or root motion extraction?

### Takeaway
The engine has weighted pose blending with 1D and 2D (barycentric) blend spaces, crossfade-free inertialization (quintic offset decay plus decaying velocity), redirect-from-display, and selection of "fitted" stop routes. Root motion extraction is full-featured (per-axis, loop wraps, blend-weighted, hand-over turns). The only IK-like feature is a look chain (spine to head, yaw and pitch limits). There is no two-bone IK, no foot locking, no foot contact data, no mirroring, and no clip events.

### Cited Findings
**Blending**
- `blend_local_transform`, `blend_local_pose`, and `blend_local_poses` (N poses, normalized weights, hemisphere-aligned rotations) — [crates/byte-engine/src/animation/blend.rs:6-86](crates/byte-engine/src/animation/blend.rs)
- `BlendSpace1D` weighs at most 2 samples; `BlendSpace2D` uses barycentric weights on triangles and clamps outside to the closest edge. `BlendWeights::MAX = 3` — [blend.rs:87-264](crates/byte-engine/src/animation/blend.rs)
- Blended clips share one phase, and the blend's duration is the weighted sum of its clips' durations — [docs/reference/animation.mdx:121-133](docs/reference/animation.mdx)

**Inertialization**
- `PoseInertializer` keeps per-node translation, scale, and rotation offsets and velocities. `begin(source_prev, source, dest_prev, dest, sample_delta, duration)` and `apply(dest, delta, out)` — [crates/byte-engine/src/animation/inertialization.rs:12-135](crates/byte-engine/src/animation/inertialization.rs)
- `settle`: the offset share is `(1−p)³(1+3p+6p²)` (a quintic that leaves the source without acceleration). The velocity term is `v·t·e^(−rate·t)` with `rate = 6.9077/duration`, so 1/1000 remains at the end — [inertialization.rs:229-239](crates/byte-engine/src/animation/inertialization.rs); rules in [docs/reference/animation.mdx:70-85](docs/reference/animation.mdx)
- The changelog records the move from a critically damped spring to the quintic offset ease. "In third-person stops the worst change in a bone's step drops from about 5 cm to under 4 cm when sprinting" — [CHANGELOG.md:57](CHANGELOG.md). Measuring offsets at the same moment cut a sprint stop's worst step from 15 cm to about 7 cm — [CHANGELOG.md:58](CHANGELOG.md)
- An anytime route that redirects a blend in progress starts "from the pose and velocity it posted last" (`begin_from_display`) — [docs/reference/animation.mdx:272-287](docs/reference/animation.mdx); [player.rs:1658](crates/byte-engine/src/animation/graph/runtime/player.rs)
- `AnimationRequest::inertialized(d)` blends a blend-position jump from the pose on screen — [CHANGELOG.md:64](CHANGELOG.md)

**Pose-fit transitions (closest existing relative of matching)**
- With `matching_frames()`, fitted alternatives compete. Eligible routes must fit every contributing source clip (weight above 0.001), and the earliest next wait frame wins — [docs/reference/animation.mdx:201-214](docs/reference/animation.mdx); [graph.rs:740-750](crates/byte-engine/src/animation/graph.rs)
- `time_until_route` reports the time to the nearest wait point — [player.rs:586](crates/byte-engine/src/animation/graph/runtime/player.rs); [CHANGELOG.md:59](CHANGELOG.md)
- Wait and entry frames count at the clip's authored frame rate; "a clip without a frame rate never reaches a frame point" (so this fails for glTF) — [docs/reference/animation.mdx:183-190](docs/reference/animation.mdx)
- `compare_animation_bone_positions(first, t1, second, t2)` returns per-bone global position distances between two resource clips. It allocates four `Vec`s per call — [crates/byte-engine/src/animation/skeletal.rs:117-168](crates/byte-engine/src/animation/skeletal.rs)

**Root motion**
- `RootMotionDelta { translation: Vector<ParentSpace>, rotation }` with `between`, `then` (loop joins), and `blend` — [crates/byte-engine/src/animation/root_motion.rs:10-52](crates/byte-engine/src/animation/root_motion.rs)
- `RootMotionSettings { node_name, translation: RootMotionTranslation (X/Y/Z bitmask), rotation: None|Full }` — [player.rs:318-378](crates/byte-engine/src/animation/graph/runtime/player.rs)
- `RootChain` samples only the root node and its ancestors. A blend measures each weighted clip's delta and mixes the deltas by weight — [player.rs:1891-1965](crates/byte-engine/src/animation/graph/runtime/player.rs); rules in [docs/reference/animation.mdx:87-113](docs/reference/animation.mdx)
- `turning(yaw)` hands a one-shot's body turn to the entity through root motion at hand-over — [graph.rs:860-870](crates/byte-engine/src/animation/graph.rs); [CHANGELOG.md:7-9](CHANGELOG.md)

**IK and related**
- `LookChainSettings { nodes: &[(&str, f32)], body_node, yaw_limit, up_limit, down_limit, response }` turns a chain toward a target on top of the pose, with easing `1 − e^(−response·Δ)` — [crates/byte-engine/src/animation/graph/runtime/player/look.rs:11-57](crates/byte-engine/src/animation/graph/runtime/player/look.rs); rules in [docs/reference/animation.mdx:446-478](docs/reference/animation.mdx)
- A grep across the animation code, resource-management, and docs for `two.?bone|fabrik|ccd|inverse kinematic|IK|foot lock|foot plant|foot contact|motion match|pose search|trajectory|mirror` found no animation IK, foot locking, motion matching, or mirroring. The only hit was a test node named "IK Helper" — [player.rs:3715](crates/byte-engine/src/animation/graph/runtime/player.rs)

### Inferences
- `PoseInertializer` plus the redirect-from-display path is already the transition primitive motion matching usually needs (inertialize from the posted pose and velocity into a new database frame). The quintic-plus-decay curve is a variant of the polynomial inertialization in the literature. The inertializer works on local `LocalTransform`s per node, so it would accept a new matched pose directly.
- Root motion extraction already measures per-clip root deltas from the root chain. The same `sample_nodes` path would compute future root positions and directions at fixed offsets for trajectory features at bake time.
- Foot locking and foot IK would need contact annotations (absent), a two-bone solver (absent), and ground queries. Ground queries lack shape casts and have one step of latency.

### Gaps
- I did not trace whether `begin_from_display` keeps the visual pose's root-motion-removed channels consistent when the destination comes from an arbitrary clip time. That matters for motion matching jumps to mid-clip frames.

## 4. Which benchmarks cover animation, what do they measure, and which job-system and math or SIMD facilities could a pose search use?

### Takeaway
`benches/animation_graph.rs` (divan) times single-player paths by skeleton size and `characters_*` crowds of 1, 64, and 1,024 with 64-node skeletons. `characters_locomotion` is the reference "typical frame": about 1.8 µs per walking character and about 0.57 ms for 1,024 on an M4. The alley job system spreads players over performance-core lanes with thread-local scratch. `std::simd` (`portable_simd`, nightly) is enabled crate-wide but used only in physics. The math crate wraps `maths-rs` with branded scalar types and has no SIMD batch types for pose or feature vectors.

### Cited Findings
**Benchmarks**
- `active_pose`, `active_root_motion`, and `inertialized_transition` take node counts `[1, 32, 128]`. `characters_root_motion`, `characters_transitioning`, `characters_switching`, and `characters_locomotion` take character counts `[1, 64, 1024]` with `CHARACTER_NODE_COUNT = 64` — [crates/byte-engine/benches/animation_graph.rs:21-64](crates/byte-engine/benches/animation_graph.rs)
- The `Locomotion` fixture "Steers through a planar blend of five clips keyed at 30 frames per second, with root motion". Durations differ, and each character starts at its own phase. A walking crowd "steers around a circle, each character facing its own way" — [crates/byte-engine/src/animation/graph/runtime/benchmarks.rs:20-45, 113-118, 244-258](crates/byte-engine/src/animation/graph/runtime/benchmarks.rs)
- Per-stage costs on an Apple M4 (October 2026, 64 nodes): sampling a 31-key clip about 0.35 µs, blending three poses 0.27 µs, global pose 0.4 µs, root chain 0.03 µs, a walking character about 1.8 µs. "Sampling was most of it" — [docs/develop/animation.mdx:49-53](docs/develop/animation.mdx)
- Crowd timing: 1,024 walkers took 0.60 ms on 4 lanes, 0.72 ms on 6, 0.58 ms on 8, and 0.62 ms on 10 — [docs/develop/animation.mdx:117-125](docs/develop/animation.mdx). The changelog states 1.8 µs (was 8.8 µs) per walker and 0.57 ms (was 10.9 ms) for 1,024 — [CHANGELOG.md:32](CHANGELOG.md)
- Project memory: `characters_locomotion` "is the typical-frame animation benchmark the user cares about". Compare saved bench binaries interleaved, in both orders, and keep them outside `target/` because the mbx cache can garbage-collect it — [memory/animation-bench-baselines.md](/Users/facundovilla/.claude/projects/-Users-facundovilla-development-Byte-Engine/memory/animation-bench-baselines.md)
- Other benches: `pathfinding` (A*, funnel, `navigation_mesh_path`), `physics`, `alley`, `tasks`, `input`, `channel` — [crates/byte-engine/Cargo.toml:30-77](crates/byte-engine/Cargo.toml); [crates/byte-engine/benches/pathfinding.rs:82-287](crates/byte-engine/benches/pathfinding.rs)

**Rules the benchmarks guard**
- Evaluation only reads the pool, graph, and binding. Results are independent of lane count and order. A state binds once. "Nothing allocates in a step for a player that has played a frame, except the transforms a pose update owns." Packed sampling equals resource sampling. Root motion samples the root chain only — [docs/develop/animation.mdx:127-155](docs/develop/animation.mdx)

**Job system (alley)**
- `Alley` API: `execute`, `execute_with_mut`, `execute_phased`, `execute_shared_mut`, `join`, and `for_each_mut(items, lanes, f)` (even partitions, calling thread runs lane 0). Lane collectives: `broadcast`, `broadcast_mut`, `each`, `each_ref`, `each_ref_keyed`, `only_one_runs`, `with_limited_parallelism`, `share`, `share_range` — [crates/byte-engine/src/core/alley.rs:261-440, 1442-1660](crates/byte-engine/src/core/alley.rs)
- Dispatches allocate nothing for their sections, never overlap, and have no leader lane — [docs/reference/alley.mdx:13-38](docs/reference/alley.mdx); [docs/develop/core/alley.mdx](docs/develop/core/alley.mdx)
- `PairBatches` and `LaneRanges` give lock-free disjoint writes for items that touch shared entries — [crates/byte-engine/src/core/alley/batches.rs:1-16](crates/byte-engine/src/core/alley/batches.rs); [crates/byte-engine/src/core/alley/split.rs:1-8](crates/byte-engine/src/core/alley/split.rs)
- Animation uses `alley.for_each_mut(players, player_lanes(n), …)` with `PLAYERS_PER_LANE = 16`, capped at `machine::performance_parallelism()` (the performance cores on Apple silicon), plus a thread-local `POSE_SCRATCH` — [player.rs:251, 444-452, 487-528](crates/byte-engine/src/animation/graph/runtime/player.rs); [crates/byte-engine/src/core/machine.rs:10-14](crates/byte-engine/src/core/machine.rs)
- Clip loading runs on an async worker (`kanal` channels) — [runtime.rs:385-409](crates/byte-engine/src/animation/graph/runtime.rs)

**Math and SIMD**
- `#![feature(portable_simd)]` is enabled in both `byte-engine` and `resource-management` — [crates/byte-engine/src/lib.rs:25](crates/byte-engine/src/lib.rs); [crates/resource-management/src/lib.rs:8](crates/resource-management/src/lib.rs)
- `f32x4` is used in the physics broad phase (sweep and prune over a grid, 4 bounds per step) and in the solver and convex hull. A grep found SIMD only in `physics/intersection.rs`, `physics/dynabit/solver.rs`, and `physics/convex_hull.rs` — [crates/byte-engine/src/physics/intersection.rs:1-40, 870-1020](crates/byte-engine/src/physics/intersection.rs); [crates/byte-engine/src/physics/convex_hull.rs:3-47](crates/byte-engine/src/physics/convex_hull.rs)
- The `math` crate ("Branded, allocation-free geometry types") depends on `maths-rs`, `rkyv`, and `serde`. It exports `Point`, `Vector`, `UnitVector`, `Orientation`, `AffineMatrix`, `AABB`, `Capsule`, and XZ helpers such as `closest_approach_time_xz`, `point_along_xz`, and `detour_corner_xz` — [crates/math/Cargo.toml](crates/math/Cargo.toml); [crates/math/src/lib.rs:1-60](crates/math/src/lib.rs)

### Inferences
- A brute-force or banded search over a normalized feature matrix fits the existing patterns. Use `f32x4`/`f32x8` structure-of-arrays loops as the physics broad phase does, a thread-local scratch as `POSE_SCRATCH` does, and per-character parallelism through `for_each_mut` inside `advance_players`. All of it is lock-free and allocation-free after warm-up, which the rules require.
- The 1.8 µs per walker budget gives a concrete bar. A per-character search must be compared on `characters_locomotion`, and a new `characters_motion_matching` case (same 64-node skeleton, 1/64/1,024 characters) would fit the bench file's conventions.
- Searching every frame for 1,024 characters on 4 performance lanes against a database of tens of thousands of frames would probably dominate the 0.57 ms crowd frame. That implies search intervals (for example every 0.1 s, as the NPC planner already uses), acceleration structures, or LOD. This is an estimate, not a measurement.

### Gaps
- Benchmarks were not run (disk constraint). All numbers are the repository's own October 2026 M4 figures.
- I did not check whether the pinned nightly toolchain supports `f32x8` codegen well on the target platforms (`rust-toolchain.toml` was not read).

## 5. What do the docs and the sandbox apps say about animation, navigation, and characters?

### Takeaway
Animation is documented in all three sections the AGENTS.md charter defines: a Reference spec (541 lines), a Contribute design note (175 lines) with rules and guards, and a Use task page (200 lines). Navigation has Reference and Use pages. Project conventions require writing behavior in `docs/reference` first, discussing the design before implementing, keeping interfaces slim, and benchmarking before and after. The `third_person` sandbox is the only consumer of skeletal animation; `isometric` uses flipbook sprites.

### Cited Findings
- [docs/reference/animation.mdx](docs/reference/animation.mdx) covers: skeletons and clips (15), sampling (38), blending (61), inertialization (70), root motion (87), graph, states, spaces, and routes (115-214), build errors (215), animators (233), requests, playback, and stepping (259-306), updates (308), queries (326), data stream (339), look chain (446), errors (480), clip pool (490), flipbooks (510), and limits (519).
- [docs/develop/animation.mdx](docs/develop/animation.mdx) covers the frame-flow table, packed clips, what players share, how a crowd advances, why crowds stay on the fastest cores, six rules a change must keep, and the tests and benchmarks that guard each rule.
- [docs/use/animation.mdx](docs/use/animation.mdx): "Build the graph", "Create the system, then the animators", "Drive it every tick" (tick-order code), "Decide from what's playing" (`matching_frames` example), "Turn the head".
- [docs/use/navigation.mdx](docs/use/navigation.mdx) and [docs/reference/navigation.mdx](docs/reference/navigation.mdx) document areas, the mesh, agents, steering, detours, debug drawing, tolerances, and limits (rectangular areas only, one level).
- AGENTS.md table: Reference pages "never" contain code, numbered steps, or rationale. Contribute pages are design notes with rules and tests. Use pages are one task each. Run `bun run check:style` from `docs-site` — [AGENTS.md](AGENTS.md)
- AGENTS.md engineering rules: "Delete first"; "Prefer pure functions"; "Separate concerns by layers"; "Avoid allocations, especially transient ones"; benchmark performance-sensitive changes before and after; black-box tests of observable behavior; `cargo nextest run --workspace` — [AGENTS.md](AGENTS.md)
- Project memory index: "Reference docs are the spec — write behavior in docs/reference first"; "Discuss design before implementing"; "Slim interfaces, no wrappers"; "Systemic fixes over patches"; "Continuous navigation over stacked heuristics … rethink when patches pile up; animation artifacts aren't steering's job" — [memory/MEMORY.md](/Users/facundovilla/.claude/projects/-Users-facundovilla-development-Byte-Engine/memory/MEMORY.md)
- `todo.md` has one animation item, "Drive multiple animations concurrently using a single animation driver" (line 59). Line 116 asks to provide "animation-safe bounds so posed meshlet culling can be re-enabled"; line 118 asks for skinned detail levels once those bounds exist — [todo.md:59, 116-118](todo.md)
- Sandbox: `third_person` (MotusMan, `MotusMan/MotusMan_v55.fbx`, `anims/MOB1_*.fbx` clips) is the animated-character app — [third_person/src/animation.rs:25, 535-541](/Users/facundovilla/development/sandbox/third_person/src/animation.rs). `isometric` characters use "a run flipbook while moving, the facing idle image once stopped" — [isometric/src/character.rs:1-28](/Users/facundovilla/development/sandbox/isometric/src/character.rs)
- Sandbox history (`git log`): "Choose the third_person NPCs' gait by the player's distance alone", "Move the third_person NPCs on rate-limited headings", "Steer the third_person characters around each other", "Give third_person's graph to the animation system".
- The changelog notes that `any_state_to` was added because otherwise a target "needed a route from each state, over a thousand in a graph with many twins" — [CHANGELOG.md:60](CHANGELOG.md)

### Inferences
- Under the charter, a motion matching feature would need: Reference sections (database and feature definition, query, search rules, transition rules, limits, data-stream fields) in `docs/reference/animation.mdx` or a new `docs/reference/motion-matching.mdx`; a Contribute design note (storage layout, search, rules, guards); a Use page ("Drive a character with motion matching"); and the Rust API links.
- The third_person graph's size (104 clips, twin states, fitted-stop tables, over a thousand routes before `any_state_to`) and the memory note "rethink when patches pile up" both point to the hand-authored locomotion graph as the part motion matching would replace. Jumps, deaths, and other one-shots could stay graph-driven.

### Gaps
- `docs/use/sample-project` and `docs/develop/core/tasks.mdx` (modified in the working tree) were not read in detail.

## 6. Does the engine have machine-learning inference, nearest-neighbor or spatial structures, or debug drawing a motion matching system could reuse?

### Takeaway
There is no ML inference and no KD-tree, BVH, or other nearest-neighbor structure. The only spatial acceleration is the physics sweep-and-prune grid and the navigation mesh's polygon location. Debug tooling is a strength: retained world-space debug meshes (sphere, box, capsule, line, circle), per-animator NDJSON debug data streams with a documented schema, and an inspector that can inject input actions headlessly.

### Cited Findings
- A grep for `onnx|tract|candle|burn::|tensor|neural|inference|mlp|ort::` across `crates` found only unrelated hits (DX12 stride inference, inertia tensors, IOKit ports). The engine's dependencies contain no ML crate — [crates/byte-engine/Cargo.toml](crates/byte-engine/Cargo.toml)
- A grep for `kd.?tree|bvh|octree|quadtree|spatial hash|nearest neighbo|knn` returned no structures. The broad phase "finds the world bounds that overlap" with sorted sweeps and an optional grid — [crates/byte-engine/src/physics/intersection.rs:1-40](crates/byte-engine/src/physics/intersection.rs). `math::AABB` exists "for broad-phase and contact queries" — [crates/math/src/aabb.rs:4](crates/math/src/aabb.rs)
- Pathfinding has `a_star`, `a_star_batch`, a retained `PathSearch` (shared working storage), and graph types `TrivialGraph` and `BitMatrixGraph` — [crates/byte-engine/src/gameplay/pathfinding.rs:1-8, 26-120, 242-381](crates/byte-engine/src/gameplay/pathfinding.rs)
- `DebugShape` covers Sphere, oriented Box, Capsule, Line, and Circle, with `DebugDepthMode`. Meshes are retained under a handle and replaced by publishing again — [crates/byte-engine/src/rendering/debug/mod.rs:1-81](crates/byte-engine/src/rendering/debug/mod.rs). Navigation uses them for the boundary, radii, and paths behind `--navigation.debug=true` — [crates/byte-engine/src/gameplay/pathfinding/navigation.rs:446-450](crates/byte-engine/src/gameplay/pathfinding/navigation.rs); [docs/use/navigation.mdx:124-135](docs/use/navigation.mdx)
- The animation debug data stream record carries `status`, `error`, `missing`, `request`, `motion` (state, phase, time, duration, clips with weights), `transition`, `pending`, `root_motion`, and `look`. It allocates nothing once the line has grown — [crates/byte-engine/src/animation/graph/runtime/player/stream.rs:1-60](crates/byte-engine/src/animation/graph/runtime/player/stream.rs); [docs/reference/animation.mdx:339-445](docs/reference/animation.mdx)
- A `DebugDataStream` is created through the world, as in `world.create(DebugDataStream::new(handle, "Character"))` — [third_person/src/player.rs:61](/Users/facundovilla/development/sandbox/third_person/src/player.rs); [CHANGELOG.md:22](CHANGELOG.md)
- Inspector messages: `TransformationUpdate`, `Delete`, `TriggerAction`, entity query, screenshots — [docs/reference/inspector.mdx:46-85](docs/reference/inspector.mdx)

### Inferences
- A motion matching search structure (KD-tree, bounding-box hierarchy over feature rows, or clustered or banded brute force) would be new code. Nothing exists to reuse besides SIMD idioms and `AABB`.
- Learned motion matching (decompressor, stepper, and projector networks) would need a small, allocation-free MLP evaluator written from scratch, or a new dependency.
- Debug views for motion matching map onto existing pieces. Predicted and matched trajectories become `DebugShape::Line` and `Circle` (as navigation draws paths). Feature costs, chosen clip and frame, and per-term costs become new fields in the animation DDS record. Headless play-testing can use `TriggerAction` on the movement action.

### Gaps
- I did not check whether the renderer's meshlet culling builds any hierarchy that could be generalized; a grep for `bvh` found none.

## 7. Which gaps would a motion matching system need filled?

### Takeaway
The engine supplies the runtime "back end" (packed sampling, a pooled arena, blending, inertialization from the posted pose, root motion, a look chain, per-lane crowd evaluation, debug streams). Every motion-matching-specific piece is missing: bake-time feature extraction, database storage, trajectory prediction, search, a matching state or player, mirroring, annotations and tags, foot IK and locking, a character controller, and tooling.

### Cited Findings
- No bake step produces per-frame data. Clips bake as f32 curves and pack only at load time — [fbx/skeleton.rs:107-166](crates/resource-management/src/asset/handler/implementations/fbx/skeleton.rs); [runtime.rs:393-406](crates/byte-engine/src/animation/graph/runtime.rs)
- The clip format has no events, tags, foot-contact channels, or velocities: an `Animation` holds only name, skeleton, duration, frame rate, and tracks — [crates/resource-management/src/resources/animation.rs:199-206](crates/resource-management/src/resources/animation.rs)
- glTF clips have no frame rate, so frame-addressed features fail for them — [docs/reference/animation.mdx:28, 183-190](docs/reference/animation.mdx)
- Pose comparison exists only as an allocating global-position diff between two resource clips — [skeletal.rs:117-168](crates/byte-engine/src/animation/skeletal.rs)
- State selection belongs to client code, and gameplay chooses states "with normal Rust control flow" — [graph.rs:6-8](crates/byte-engine/src/animation/graph.rs). Motions are `Clip`, `RoundRobin`, `Linear`, and `Planar` only — [crates/byte-engine/src/animation/graph/motion.rs:8-65](crates/byte-engine/src/animation/graph/motion.rs)
- A request carries a state plus an optional blend position and inertialization duration. Nothing in it carries a trajectory or a desired velocity — [motion.rs:321-345](crates/byte-engine/src/animation/graph/motion.rs); [docs/reference/animation.mdx:259-270](docs/reference/animation.mdx)
- The system never moves the entity; gameplay applies root motion — [system.rs:68-72](crates/byte-engine/src/animation/system.rs); [docs/reference/animation.mdx:317](docs/reference/animation.mdx)
- No character controller; kinematic bodies behave as static; no shape or sweep queries — [docs/reference/physics.mdx:15-19, 130](docs/reference/physics.mdx)
- No IK solver, foot locking, or mirroring (grep results in section 3).
- Navigation's root-motion "response time" was removed — commit `e7e6b05d8` message.

### Inferences
Each gap, with where it would likely plug in. All of these are inferences from the code structure, not from any existing plan in the repository.
1. **Feature extraction at bake time.** Add a new resource class (for example a pose database) baked by `resource-management` from a list of clips, perhaps named in a `.bead` sidecar. It would sample each clip at a fixed rate with the shared `math::interpolate`/`KeySpan` path. Features: chosen bone positions and velocities in character space; future root positions and facings at fixed offsets, from root-chain sampling; per-feature normalization statistics. Store it as an rkyv binary payload (structure of arrays, 16- or 32-byte aligned) rather than metadata. It should live in its own layer, separate from runtime search, per AGENTS.md "Separate concerns by layers".
2. **Database storage and residency.** Either a second arena in or beside `AnimationPool` (same byte-budget and eviction rules), or a resident database the clips' `TrackTargets` bindings index into. The database's clips still need to be resident packed clips for pose output.
3. **Trajectory prediction.** A pure function from intent (desired velocity and facing, from input or from `Navigation::steer` plus the path polyline) and current root state to future samples, for example a critically damped spring. It would replace the sandbox's ad hoc `Motion` easing and NPC turn-rate code. It probably belongs on the gameplay side, with the request carrying the trajectory, which keeps the "system never moves the entity" rule.
4. **Search.** SIMD brute force or banded/AABB-pruned search over normalized features, run inside `advance_players` per lane with thread-local scratch. Search at an interval or on events such as a large trajectory change. Results must stay independent of lane count (rule 2) and allocate nothing (rule 4).
5. **Integration with the graph.** Either a new `StateMotion` variant (a "matched" state that plays database frames, with graph routes still handling jumps, deaths, and turns) or a separate player kind in `AnimationSystem`. Requests would gain a trajectory payload. The memory notes favor slim interfaces and discussing the design first.
6. **Transitions.** Reuse `PoseInertializer` with the redirect-from-display path (`begin_from_display`) when the search switches frames. The root-motion delta across a switch must stay continuous, as existing redirects already promise ("redirected blends keep root motion continuous", test at [player.rs:3152](crates/byte-engine/src/animation/graph/runtime/player.rs)).
7. **Annotations and tags.** Add per-frame tags (gait, stance, no-enter or no-exit ranges, foot contacts) and mirroring, none of which exist. They could replace the hand-measured `STOP_ANIMATION_RESOURCES` frame tables.
8. **Post-processing.** Foot locking and two-bone IK (none exist), ground adaptation (needs ray queries, which answer one step late, or a new synchronous query), and stride or orientation warping to reconcile root motion with steering.
9. **Character controller.** Optional, but needed for a simulation-bound variant. `Kinematic` bodies are currently static.
10. **Debug views and tooling.** DDS fields for the query, best cost, per-feature costs, and chosen clip and frame; `DebugShape::Line` and `Circle` for predicted and matched trajectories; an inspector query of database stats. There is no editor; graphs are code-authored, so a tuning workflow would likely be parameters (the `ConfigurationPort` or parameter scopes) plus DDS.
11. **Benchmarks and tests.** A `characters_motion_matching` divan case beside `characters_locomotion`, a bake benchmark, black-box tests (for example "a query built from frame k of clip c finds frame k of clip c"), and doc-aligned rules in `docs/develop/animation.mdx`.

### Gaps
- The repository has no plan, todo entry, or issue for motion matching; the only related todo is "Drive multiple animations concurrently using a single animation driver" — [todo.md:59](todo.md).
- I could not measure how large a database for the 104-clip MotusMan set would be, or how long a search would take, without building and running code.
