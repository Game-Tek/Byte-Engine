# Classic (non-neural) motion matching: algorithm and industry development

Scope: classic motion matching, from motion graphs to production practice as of October 2026. Neural successors, search acceleration, transitions and post-processing (inertialization, warping, foot locking), implementation lists, engine surveys, datasets, and patents belong to other researchers' notes. They appear here only for context.

Primary-source note: the three most detailed primary sources used here were read directly, not through summaries.
- Simon Clavet's GDC 2016 slide deck (PDF text extracted).
- Kristjan Zadziuk's GDC 2016 deck, including the presenter notes embedded as PDF comments.
- Naughty Dog's GDC 2021 *The Last of Us Part II* deck, read slide by slide with speaker notes from the .pptx, without downloading the whole 4 GB file.

Daniel Holden's reference implementation (`orangeduck/Motion-Matching`, files `database.h` and `controller.cpp`) was also read at source level. Numbers quoted from these sources are exact.

---

## 1. Origins: from motion graphs to motion matching, and what it replaced

### Takeaway
Motion matching is the games industry's brute-force, greedy simplification of 2002-era motion graphs and 2010 Motion Fields. It keeps one idea from Motion Fields: "we can jump to any other frame whenever we want." It drops the graph construction and the reinforcement-learning planner, and replaces them with a per-frame (or per-few-frames) nearest-neighbour search for the frame whose pose and future trajectory best match a code-simulated plan.
- Michael Büttner (Ubisoft Toronto, nucl.ai 2015) originated it.
- Simon Clavet (Ubisoft Montreal) productized it for *For Honor* (2017).
- Kristjan Zadziuk (Ubisoft Toronto) presented it alongside Clavet at GDC 2016.

It promised to replace hand-built state machines and blend trees with "data + goal + matching." The selling points were higher quality, controllable responsiveness, and minimal manual work.

### Cited Findings

**Academic precursors (2002)**
- **Motion Graphs (Kovar, Gleicher, Pighin, SIGGRAPH 2002).** The method builds a directed graph whose edges are original clips plus automatically generated transitions. Motion comes from graph walks. Details:
  - Frame distance D(i, j) is a weighted sum of squared distances between skeleton-driven point clouds (ideally a downsampled mesh), compared over windows of k frames.
  - The windows "effectively incorporate derivative information." k is "about a third of a second."
  - An optimal rigid 2D alignment (rotation about the vertical axis plus floor translation) is solved in closed form before comparison.
  - Candidate transitions are local minima of D below a user or empirical threshold, which trades transition quality against connectivity.
  - Transitions are built by linearly blending k frames after alignment.
  - The graph is pruned to its largest strongly connected component to remove dead ends and sinks.
  - Path synthesis uses branch-and-bound search over graph walks.
  - For about 6,000 frames (200 s) of data, computing candidate transitions took about 25 minutes.
  - Source: [Kovar et al. 2002, PDF](https://graphics.cs.wisc.edu/Papers/2002/KGP02/mograph.pdf)
- **Arikan & Forsyth, "Interactive Motion Generation from Examples" (SIGGRAPH 2002).** Motion is cut and pasted from mocap clips. Choosing clips is treated as a combinatorial problem solved by randomized search over a hierarchy of graphs, in real time with constraints. Sources: [SIGGRAPH history entry](https://history.siggraph.org/?p=118435); [paper PDF](https://alecjacobson.com/seminar/arikan-and-forsyth-2002.pdf)
- **Lee, Chai, Reitsma, Hodgins, Pollard, "Interactive Control of Avatars Animated with Human Motion Data" (SIGGRAPH 2002).** The method preprocesses unlabeled mocap into a graph of plausible transitions and clusters it to speed up search. Avatars are controlled by menu, sketched path, or performance in front of a camera. Sources: [SIGGRAPH history entry](https://history.siggraph.org/?p=118436); [SNU project page](https://mrl.snu.ac.kr/research/ProjectAvatar/avatar.html)

**Motion Fields (Lee, Wampler, Bernstein, J. Popović, Z. Popović, SIGGRAPH Asia 2010)**
- A "motion state" m = (x, v) is a pose plus a velocity computed from two successive poses. The motion field maps each state to a set of possible motions, built from k-nearest-neighbour states in the database. — [Lee et al. 2010, PDF](https://homes.cs.washington.edu/~jovan/papers/lee-2010-mf.pdf)
- Each frame the controller picks one action, follows it for one frame, and repeats. Reinforcement learning (an MDP with a value function stored at database states and interpolated) chooses among the actions. As a result, "a character can immediately respond to new user commands rather than waiting for pre-determined transition points as in a motion graph." — [Lee et al. 2010](https://homes.cs.washington.edu/~jovan/papers/lee-2010-mf.pdf)
- The unoptimized implementation ran at "approximately 200 frames per second." The paper names heavy use of k-NN lookups and interpolation as the extra runtime cost compared with graph methods. — [Lee et al. 2010](https://homes.cs.washington.edu/~jovan/papers/lee-2010-mf.pdf)

**How Clavet tells the story (GDC 2016 slides)**
- Clavet's history runs from `PlayAnim()` through state machines, blend/decision trees, and parametric blends (speed, strafe angle, slope) to motion graphs ("Preprocess: figure out when you can jump to somewhere else") and Motion Fields. — [Clavet GDC 2016 slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- His motivating questions:
  - "Where would you put this animation in your structure? Start-Strafe90-TurnOnSpot45-Stop."
  - "Is there a way to deal with loops and transitions in a uniform way?"
  - "For comfortable control, we need a dense graph."
  - "The problem with Motion Fields: the equations are scary… Let's just use the main idea of the paper: We can jump to any other frame whenever we want."
  - Source: [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- His definition: motion matching is "A ridiculously brute-force approach to animation selection. Algorithm: Every frame, look at all mocap and jump at the best place." Every candidate jump point has a cost. The cost is zero if the candidate matches the current situation and "the piece of motion that follows brings us where we want." — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- Conclusion slides: it is "a simple idea, that helps us reason about movement description and control (DATA, GOAL, MATCHING)." It is also a new type of animation system with "High quality, Controllable responsiveness, Minimal manual work." Final thought: "Let's just markup the mocap with the inputs that should trigger the moves. And generate everything automatically…" — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- The GDC Vault entry is "Motion Matching and The Road to Next-Gen Animation" (Clavet, Programming track, GDC 2016). — [GDC Vault](https://www.gdcvault.com/play/1023280/Motion-Matching-and-The-Road)

**Büttner (nucl.ai 2015) and Zadziuk (GDC 2016)**
- The earliest talk is Michael Büttner and Simon Clavet, "Motion Matching – The Road to Next Gen Animation," nucl.ai 2015 ([video](https://www.youtube.com/watch?v=z_wpgHFSWss)). It is cited this way in [Holden et al., Learned Motion Matching (2020)](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf). JC Delannoy calls Büttner's nucl.ai 2015 talk the first talk on motion matching in a AAA game ([Game Developer, Jan 2017](https://www.gamedeveloper.com/programming/most-inspiring-game-animation-tech-talks-of-2016)).
- Zadziuk's notes credit "Animation Programmer, Michael Buttner who's idea Motion Matching had been in the first place." — [Zadziuk GDC 2016 slides with presenter notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- Zadziuk's talk is "Animation Bootcamp: Motion Matching: The Future of Games Animation...Today" ([GDC Vault](https://www.gdcvault.com/play/1023478/Animation-Bootcamp-Motion-Matching-The)). It showed a prototype gym whose assets "do not represent any game currently in development at Ubisoft." — [Zadziuk slides](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- Zadziuk's account of the problem it replaced, from his presenter notes:
  - "we have to add nodes, setup rules, blend times, loop points, idles..."
  - "it can be hard to get characters to respond quickly and realistically to user input."
  - His example is a weight shift during a direction change. "Solving this in a state machine would be difficult as we would likely need to use a transition animation… simply blending from one direction to another would not look great and would likely slide."
  - Source: [Zadziuk slides/notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- Zadziuk's four goals were Realism, Control, Simplify, and Variety. His view of where motion matching came from: "has been around for a while and has been known as a few different things, such as motion graphs and motion fields, but it has only really been this generation of consoles that have made it possible for practical use." — [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- First test: "All data in one file, Around 10/15 minutes of untouched data." Exported "straight to the engine without any touch up." Result: "100% player controlled, Immediate response, No IK yet and foot-sliding is minimal." — [Zadziuk slides/notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- DualShockers' recap of the GDC 2016 panel reports that a locomotion system was built in four hours. It says this would take "a top Ubisoft team at least three months." — [DualShockers](https://www.dualshockers.com/ubisoft-introduces-motion-matching-the-first-step-to-next-gen-animation/). This is a secondary source and the claim is not in the slide text.

**For Honor as the first shipped player-control use**
- *For Honor* shipped motion matching. Game Anim's write-up of Clavet's talk says it had about 10 matched factors and that it ran on more than 100 on-screen characters using LOD. — [Game Anim, "Motion-Matching in Ubisoft's For Honor", May 3 2016](https://www.gameanim.com/?p=13538)
- Clavet also gave "Motion Matching For Realistic Animation in For Honor" at nucl.ai 2016 ([video](https://www.youtube.com/watch?v=4pdcA3mhe0E)), summarized in [Game Developer](https://www.gamedeveloper.com/programming/most-inspiring-game-animation-tech-talks-of-2016).
- In a 2019 exchange, Bobby Anguelov reportedly credited IO Interactive and Michael Büttner with pioneering the technique for crowds, and called *For Honor* "the best known/first game with it for player control." I saw this only in a search-result summary of his X posts, not in the post itself. Treat it as unverified.

### Inferences
- The lineage explains the main design choice. Motion graphs precompute where you may jump. Motion Fields allows jumps anywhere but plans with RL. Motion matching allows jumps anywhere and replaces planning with a cost on how far the next piece of motion travels along a code-predicted trajectory. That makes the future-trajectory feature a cheap stand-in for a value function.
- Kovar's windowed point-cloud metric (about 1/3 s, which captures derivatives) is the ancestor of matching pose positions plus velocities instead of only instantaneous pose.

### Gaps
- I could not get a transcript or slides of Büttner's nucl.ai 2015 talk. Only the video link (cited by LMM) is confirmed, so its exact feature set and numbers are not in these notes.
- I did not watch the GDC Vault recordings of Clavet's or Zadziuk's talks. Content spoken but not on slides or in notes is missing.

---

## 2. Feature vector design, normalization, cost, search cadence, continuation bias, tags, mirroring, loops, sync markers

### Takeaway
The canonical locomotion feature vector (Clavet 2016, made precise by Holden 2020) is a small set of root-relative quantities:
- **Pose:** foot positions, foot velocities, hip velocity.
- **Future trajectory:** 2D positions and facing directions at about 1/3, 2/3, and 1 s.

Each feature group is z-normalized by its dataset standard deviation, scaled by a user weight, and compared by squared Euclidean distance. The search runs on a timer (about 0.1 s in Holden's reference code; "every frame" at Ubisoft and Naughty Dog). It is also forced when the input changes sharply or the current clip ends.

Continuation is biased by evaluating the current frame first, adding a transition cost or bias to other candidates, and excluding nearby frames. Production systems added more matched quantities, such as a past-trajectory sample, spine facing, turn speed, stance, weapon or hand positions, and event timings. They also added cost-bias "groups" and "layers" instead of hard state logic.

### Cited Findings

**Clavet / For Honor features**
- "Trick 1: Posematch only a few bones." The pose/velocity features are "Local velocity, Feet positions, Feet velocities, Weapon position, Etc.," and they are "Precompute[d] and save[d] with the animation for fast cost computation." — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- "Trick 2: Just check where a piece of animation brings you if you play it." The trajectory-matching features are "Future position, Future orientation, Future velocity/acceleration." A slider trades "Realism" against "Comfort." — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- "Future Stance Matching" (left or right stance): the candidate's predicted future pose is compared with what gameplay wants. The result "Elegantly find[s] transitions when they exist." — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- *For Honor* matched about 10 factors (a few bones, local velocity, feet position and velocity, weapon position). Feet were ignored for some motions such as turns on the spot. When no good match existed, the system fell back to blending. — [Game Anim For Honor write-up](https://www.gameanim.com/?p=13538)
- Search cadence. Clavet: "Every frame, look at all mocap and jump at the best place." A slide on a "very tired" style says the system may "Switch multiple times per second." — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf). Listed optimizations: "LOD, KD-Tree, Motion Shaders." — same source

**Zadziuk / Ubisoft Toronto**
- Features: "Root Position / velocity; Past and Present trajectory; Joint Positions / velocity; Tags." These describe a small set of criteria over a set time, which the notes give as 1 second. — [Zadziuk slides/notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- "Motion Shaders" were a node graph that defined the cost functions for matching current, future, and past trajectories of limbs or the root.
  - Example from the notes: "positions should 10% important, velocity 30% and hands could be 80% important."
  - Sliders from 0 to 1 traded "respecting the positions in the animation or favour player input."
  - Designers could also "adjust the amount of dampening on the input of the stick."
  - Source: [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- Tagging: "simple tagging of areas of interest or waste, such as idles or garbage… we could start to highlight areas we wanted to favour." — [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- Mirroring "didn't work for some reason… it seemed to see too much data to make a clear choice." — [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf). This is 2016 practice; later systems mirror routinely (see below).

**Holden et al., Learned Motion Matching (SIGGRAPH 2020), section 3 "Basic Motion Matching"**
- The paper describes "our own state-of-the-art implementation used in several AAA game productions." Its loop: "every N frames we search a database of animation for a frame which best matches some set of features… If a frame with a lower cost than the current frame is found, a transition is inserted." — [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)
- The locomotion feature vector, defined "in a similar way to Clavet [2016]," is x ∈ ℝ²⁷:
  - 2D future trajectory positions projected on the ground, "20, 40, and 60 frames in the future (at 60Hz)," local to the character (6 values).
  - 2D future facing directions at the same times (6).
  - Two foot positions (6).
  - Two foot velocities (6).
  - Hip velocity (3).
  - Source: [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)
- Normalization: "we scale each feature (e.g. left foot position) by its standard deviation in the data-set. This scaling can then be tweaked further by a user weighting… however we found that in most cases a default weighting of 1 is sufficient." — [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)
- Query: "At runtime, every N frames, or when the user input changes significantly, a query vector x̂ is constructed." Pose features "can typically be extracted from the entry in the Matching Database X corresponding to the current frame i." Trajectory features come from "a spring-damper-based system which generates a desired velocity and direction based on the current state of the joystick [Kermse 2004]." — [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)
- Cost: k* = argmin_k ‖x̂ − x_k‖². If k* ≠ i, set i := k* and insert an inertialization transition [Bollo 2016, 2017]. Each frame, i := i + 1. — [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)
- Tags and gaits: "Either these can be added to the feature vector x as one-hot encoded vectors, or alternatively, ranges in the Animation and Matching Databases can be pre-computed, and the search limited to these ranges when certain gaits or tags are required." — [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)
- Acceleration options, for context only: KD-tree (Clavet 2016), a voxel lookup table (Büttner 2018), clustering, or Holden's two-layer AABB hierarchy over 16- and 64-frame groups. PCA is optional. — [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)

**Holden's open-source reference implementation (exact values from source)**
- Feature layout (27 floats): LeftFoot position (3), RightFoot position (3), LeftFoot velocity (3), RightFoot velocity (3), Hips velocity (3), trajectory positions 2D (6), trajectory directions 2D (6).
  - Every feature is expressed relative to the root (simulation) bone: `quat_inv(root_rot) * (p − root_pos)`.
  - Trajectory samples are at frame offsets +20, +40, +60, clamped to the end of the clip's range.
  - Directions are the root's +Z axis at those frames, keeping only the x and z components.
  - Source: [database.h](https://github.com/orangeduck/Motion-Matching/blob/main/database.h)
- Normalization (`normalize_feature`), per feature group:
  - offset = per-dimension mean.
  - std = mean over the group's dimensions of the per-dimension standard deviation. The whole group shares one std, which preserves its geometry.
  - scale = std / weight.
  - stored value = (x − offset) / scale.
  - The query is normalized with the same offset and scale.
  - Source: [database.h](https://github.com/orangeduck/Motion-Matching/blob/main/database.h)
- Default weights: foot position 0.75, foot velocity 1.0, hip velocity 1.0, trajectory positions 1.0, trajectory directions 1.5. The README notes the database must be regenerated when weights change. — [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp); [README](https://github.com/orangeduck/Motion-Matching)
- Search (`motion_matching_search` / `database_search`):
  - The cost of the current frame is computed first and is the bar to beat.
  - Every other candidate starts at `transition_cost` (default 0.0) and adds squared differences, with early-out once the running cost exceeds the best.
  - The search skips the last `ignore_range_end = 20` frames of each clip range and any frame within `ignore_surrounding = 20` frames of the current one.
  - A transition happens only if `best_index != frame_index`.
  - Source: [database.h](https://github.com/orangeduck/Motion-Matching/blob/main/database.h)
- Search cadence (`controller.cpp`):
  - `search_time = 0.1f` s at a fixed `dt = 1/60`.
  - A search also runs when `end_of_anim` is true (the next frame would clamp to the range end).
  - A forced search fires when the desired-velocity change rate `(v_curr − v_prev)/dt` or the desired-rotation change rate (scaled angle-axis / dt) was ≥ 50.0 on the previous frame and is < 50.0 now. That is, the forced search fires when a rapid input change finishes. A `force_search_timer` of 0.1 s rate-limits it.
  - Source: [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp)
- Query construction:
  - Pose features are copied (denormalized) from the database row of the currently playing frame, not measured from the blended output pose.
  - Trajectory features come from the predicted simulation trajectory (4 samples: now, +20, +40, +60 frames), expressed relative to the character entity's current root position and rotation.
  - Source: [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp)
- "There are no tags used to disambiguate walking and running." Gait is implied by the trajectory speeds. — [README](https://github.com/orangeduck/Motion-Matching)

**Naughty Dog, *The Last of Us Part II* (Mach & Zhuravlov, GDC 2021): production feature design**
- Their recap of the loop:
  - "A piece of code we call Motion Model provides desired trajectory based on move stick extrapolation or AI pathing."
  - "Every frame – all animations in the set and every single frame in them are being searched to find the best match in terms of pose and trajectory."
  - Then the system plays it, applies procedural adjustments, and repeats.
  - Source: [ND GDC 2021 deck (.pptx)](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx); [ND GDC 2021 page](https://www.naughtydog.com/blog/naughty_dog_at_gdc_2021)
- Pose sample: "We match only data with significant impact on the motion."
  - Ankles "to keep a continuous gait cycle."
  - Spine "to capture the body weight shifting and posture height."
  - Bipeds use both ankles and the top spine joint. Quadrupeds use all four feet and one spine joint.
  - Sample data: joint position, joint velocity, and a facing axis ("typically a top spine joint forward axis").
  - Variants: wrists for swimming; for Ellie's cover, "the on-the-wall hand position and velocity."
  - "ankle velocities are sensitive even after normalization and we usually weight them low."
  - Source: [ND deck slides 12–13 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Trajectory sample: "Enough trajectory points to capture a simple shape: 3 samples up to 1.8s in the future, 1 sample up to 0.7s in the past." Sample data: Align position, Align velocity, Align direction, Align turn speed. "Align" is ND's root/capsule node. — [ND deck slide 14](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Normalization: "Typically, velocity values are much larger than position values. We normalize our data ranges… Gives us a normal distribution where σ = 1." The notes add: "since the type of normalization we use accounts for majority of data, one broken animation won't screw up the whole set." — [ND deck slide 30](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Weight structure: "Too many numbers. The numbers change when the data change. We group individual weights into Pose or Goals groups. Master multiplier for each group." — [ND deck slide 29](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Player weighting strategy:
  - "High pose weight (1.3-1.5 times of trajectory weight)."
  - "Compulsory use of previous trajectory goal (0.4-0.7 sec)."
  - "Medium-far future trajectory (1.2-1.8 sec)."
  - "Low current velocity weight."
  - "Low grouping bias, medium continuous playback bias."
  - "Transitions get dropped by absence of future samples, rather than by the cost."
  - Notes: the past-trajectory sample "creates a difference between the corner and start right. Between start forward and quickplant forward." Low velocity weight is used because "High velocity at current frame cost brings a lot of shredding."
  - Frames without enough future samples "are unpickable," so a transition plays to its cut and then MM picks the next match (normally the loop). The result is a "completely stateless condition less system."
  - Source: [ND deck slide 37 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- NPC weighting strategy:
  - "Short to medium trajectory length (0.5 – 1.2 sec)."
  - "High trajectory weight (1.2 - 1.5 times the pose weight)."
  - "High directional weight."
  - "Low Natural Bias to not hamper the movement precision."
  - New features: "Yaw Speed Weight" (turning in progress versus done turning), "Interim Directional Weight" (stopping precision), and "Stopping Face Distance" (when to start rotating the facing vector before the goal).
  - Notes: "We try to limit the trajectory length, so it has only one shape at a time."
  - Source: [ND deck slide 39 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Continuation and grouping biases:
  - "Natural Bias": "There's a high chance we want to keep playing what we're currently playing. Biases next frame of the currently playing animation. Helps reduce animation skipping. Cannot be too high otherwise animations feel too 'sticky'."
  - "Core Loops Group": "A list of animations that we prefer to play when input is steady."
  - "Moving Group": a list of animations where the character "wants to be (keep) moving." For the player it is enabled from Motion Model speed "that is further sprung to filter out temporal speed changes." For NPCs it comes from higher-level locomotion-state logic. It exists because the system "kept confusing stopping and direction changing animations because piece of their trajectory is very similar." Speaker note: "If something doesn't work well enough, we put it on a spring."
  - Source: [ND deck slides 31–34](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- "Custom User Layers" are cost-penalty layers used as soft tags.
  - Wade loops sit in a layer with a default penalty of 1000, "so they can never trigger." On entering water, the normal loops get the penalty and the wade loops have it removed "and even slightly biased." Idles and transitions keep coming from the base set.
  - The same mechanism handles stairs, slopes, slow sprints, and an "MM Idle Recovery" layer that is un-penalized for about a second after landing.
  - Source: [ND deck slides 35, 51 notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Foot phase and mirroring: "All moves have left foot and right foot variants; we just penalized the 'wrong foot' version so they play less. We can mirror animations and technically double the dataset size." Swimming, some traversal action packs, and some animal sets were mirrored. — [ND deck slides 23–24](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- External goals: in Traversal Action Packs v2.0, "External Goals in Motion Matching" are "AP Locators as query data (position only)," and gameplay code supplies the runtime value. A constraint moves the character over time to minimize the error between the external goal and the clip's AP-reference locator. — [ND deck slide 70](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Sync and phase: the NPC Entry System marks up "matching loop" clips in all move sets. It computes "the closest matching phase of a loop animation," computes the translation error from the phase delta, and moves the Align toward the ideal distance from the entry point every frame. Without it, NPCs arrive "with a completely opposite foot phase." — [ND deck slides 55–58](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)

**Event and timing constraints (Clavet / EA)**
- Events "are placed at the *most significant* moment in the move." A clip can request: "I want this event to happen exactly 0.8 seconds after I start." "Event constraints [are] matched in gameplay goal," and events have a type the clip requests. — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf). *For Honor* tags included "Tired" and "Heavy Attack," and outcomes such as Block, Miss, Parry, and Hit wall. — [Game Anim](https://www.gameanim.com/?p=13538)
- EA *UFC 3*'s "Real Player Motion" (RPM) included "an RPM Event System that adds timing constraints to motion queries." It also covered path generation and manipulation "to achieve better coherence between the captured motion data and the motion requests going into RPM database query." — [GDC 2018 Vault abstract, Harrower](https://gdcvault.com/play/1025228/Real-Player-Motion-Tech-in)

**Unreal Engine Pose Search (UE 5.4 to 5.8 documentation, current as of Oct 2026)**
- **Schema.** A Pose Search Schema ships with a Trajectory channel and a Pose channel, and the Pose channel defaults to `foot_l` and `foot_r`.
  - Trajectory samples have an Offset in seconds (negative means past), Flags, and a Weight. The docs advise using as few samples as possible.
  - Pose channel bones have flags (Velocity, Position, Rotation, Phase) and a weight.
  - Other channels: Heading, Position (with a sample time offset and an origin bone), Velocity, Phase, Group, and Padding.
  - Component stripping options: None, Strip XY, or Strip Z.
  - Data Preprocessor options: None, Normalize, or Normalize Only by Deviation. "Normalize" normalizes against deviation and makes the weights a unit vector.
  - Example weights in the docs: Pose 1.0 and Trajectory 3.0.
  - Source: [Epic, Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)
- **Input query pose** per channel: "Use Character Pose, Use Continuing Pose, or Use Interpolated Continuing Pose." This exposes the same choice as Holden's database-row pose query versus the actual output pose. — [Epic docs](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)
- **Database biases.** "Continuing Pose Cost Bias" (negative makes continued playback more likely), "Looping Cost Bias" (added to all looping assets), and Tags. Notify states can override the base cost bias and the continuing pose cost bias per frame range. Other notifies: Block Transition, Exclude From Database, Motion Matched Branch In, Sampling Attribute. A "Normalization Set" groups several databases (for example walk and run) so they are normalized together. — [Epic docs](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)
- **Motion Matching node.**
  - "Search Throttle Time" sets the minimum time between searches. Per the API docs, locomotion searches every update by default, while cases such as jumping can search less often.
  - "Pose Jump Threshold Time" blocks jumps within the same clip unless the target is at least that far from the current time.
  - "Pose Reselect History" blocks re-picking poses selected within the last N seconds, across all segments.
  - The node also has blend time and profile, Use Inertial Blend, and Max Active Blends (0 disables the blend stack).
  - Sources: [Epic docs](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine); [AnimNode_MotionMatching Python API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AnimNode_MotionMatching)
- **Mirroring** is built in through a Mirror Data Table on the schema. — [Epic docs](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)

### Inferences
- The industry baseline has stayed stable since 2016. A Byte-Engine v1 can be closely modelled on Holden's 27-d layout (foot positions and velocities, hip velocity, three future trajectory positions and directions at about 0.33, 0.67, and 1.0 s, root-relative, 2D on the ground plane). Add the ND additions as optional channels: one past-trajectory sample at about 0.4–0.7 s, a spine facing axis, and turn speed. A schema made of typed channels (UE's model) generalizes this without code changes.
- Normalize by group, not by dimension. One std per feature group (Holden), or a robust std so outliers don't dominate (ND), keeps a 3D position's geometry intact. Expose weights as group master multipliers (ND) so tuning survives data changes.
- Continuation bias appears in every system, built from up to three parts:
  - Evaluate the current frame first, so it is the bar a switch must beat.
  - Add a constant transition cost or continuing-pose bias.
  - Exclude a neighbourhood of the current frame (Holden ±20 frames; UE Pose Jump Threshold Time) and recently chosen frames (UE Pose Reselect History).
- "Transitions drop out when their future samples run out" is a design rule shared by Holden's `ignore_range_end` (20 frames) and ND. It makes clip cut length a precise authoring control over handover time, with no state logic.
- Soft tags as additive cost layers (ND's 1000-cost penalty, UE's cost-bias notifies) are more composable than hard filtering. Hard filtering (LMM's precomputed ranges, UE Choosers selecting databases) is cheaper and more predictable. Production systems use both.

### Gaps
- Epic's documentation does not publish default trajectory sample offsets or weights for the Game Animation Sample's schemas. Those live in the sample assets, which I did not inspect.
- ND does not give its exact cost formula, for example whether facing is compared as a vector or an angle, or how turn speed is scaled.
- *For Honor*'s exact trajectory sample times and weights were not found in the slides or write-ups.

---

## 3. Desired trajectory and prediction: springs, simulation versus character entity, root motion, desync correction, navigation-driven queries

### Takeaway
Classic motion matching splits the character into two parts:
- a **code-driven simulation object**: a capsule or point moved by a critically damped spring on velocity and facing, which also produces the predicted future trajectory;
- a **data-driven character entity**: moved by the selected clip's root motion.

The search keeps the two together by choosing clips that follow the simulation. Any remaining drift is corrected with a damped adjustment and a hard radial clamp. Clavet and Holden both used a 15 cm clamp. ND used no clamp for the player, so the player is effectively always code-driven, and 0.1–0.6 m for NPCs.

AI and navigation drive the same query by replacing stick extrapolation with path following. Production found that paths need shorter trajectory windows and explicit slow-down at corners.

### Cited Findings

**Clavet / For Honor**
- "Trajectory Simulation Choices: Displacement from Animation? Displacement from Simulation? Our choice: The code decides the real simulation point."
- "Spring damper on velocity: predictable and comfortable."
- "Choose animations that follow the simulation, And do Displacement from Animation. Animation moves and rotates the entity. Clamp the entity 15 cm around the simulation."
- "Future Trajectory Prediction: Run actual simulation code to predict future trajectory. Predict stops on walls and ledges."
- Source: [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- Game Anim's write-up adds three points. There is a delay of around 1 m for stopping and ramping up from a start. "The goal is not to be as responsive as possible, the goal is to be as predictable as possible." Rotation correction is applied over time. — [Game Anim](https://www.gameanim.com/?p=13538)
- Procedural rotation: "Let the anim decide rotation but correct to match future desired position, or to match future desired orientation." "Rotate over time to match the desired future position." When future orientation matters more, rotate to match it instead. — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- Speed mismatch: "Timescale the animation according to error ratio," but "Just sliding often gives better results." — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf). *For Honor* designers were told not to timescale beyond 10% faster or 20% slower. — [Game Anim](https://www.gameanim.com/?p=13538)
- Pipeline:
  - "At runtime Gameplay makes a request (desired trajectory and event constraints)."
  - "The animation system continuously finds the best piece of data to play."
  - "We modify the result to precisely match gameplay and environment."
  - Game logic is a "Simple timed state-machine," and it is "Possible to play the game without animation! (just debug display)."
  - Source: [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)

**Holden: critically damped springs ("Spring-It-On")**
- `halflife_to_damping(h) = 4·ln2 / (h + ε)` (the 4 is described as a "fudge factor"). — [Spring-It-On](https://theorangeduck.com/page/spring-roll-call)
- Character controller: the spring's position is the character's velocity, so the spring's velocity is the acceleration. Position is the closed-form integral:
  - y = d/2, j0 = v − v_goal, j1 = a + j0·y.
  - x' = e^(−y·dt)·(−j1/y² + (−j0 − j1·dt)/y) + j1/y² + j0/y + v_goal·dt + x.
  - v' = e^(−y·dt)·(j0 + j1·dt) + v_goal.
  - a' = e^(−y·dt)·(a − j1·y·dt).
  - Source: [Spring-It-On](https://theorangeduck.com/page/spring-roll-call)
- Prediction evaluates that closed form at each future time i·dt independently ("does not simulate the steps in between"). The article says this is the method used in Learned Motion Matching. — [Spring-It-On](https://theorangeduck.com/page/spring-roll-call)

**Holden: gamepad to desired velocity**
- `desired_velocity_update` rotates the stick by the camera azimuth into world space, converts it to the simulation's local frame, and scales it by separate forward, side, and back speeds (forward or back chosen by the sign of local z). It then rotates the result back to world space. — [Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- Desired facing:
  - In strafe mode it comes from the right stick, or from the camera forward direction if the right stick is idle.
  - Otherwise it follows the desired velocity direction while the left stick is above 0.01.
  - Otherwise it holds.
  - Source: [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp)
- Tuned values:
  - Stick deadzone 0.2.
  - `simulation_velocity_halflife = 0.27`, `simulation_rotation_halflife = 0.27`.
  - Run speeds forward/side/back 4.0 / 3.0 / 2.5 m/s; walk 1.75 / 1.5 / 1.25 m/s.
  - Walk/run gait blended by a spring with a 0.1 s halflife.
  - Future desired velocities use the predicted future camera azimuth and future trajectory rotations.
  - Trajectory positions are predicted with obstacle collision, so the prediction stops at walls (like Clavet).
  - Source: [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp)
- These speeds came from measuring the data. Running peaks "just over 4 m/s" forward, "just under 3 m/s" sideways, and about 2 m/s backward. Walking is about 1.75 / 1.5 / 1.25 m/s. The simulation's velocity, acceleration, and angular-velocity histograms were compared against the animation's. — [Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- A "simulation bone" is added to the data as the root and generated procedurally:
  - Position: an upper spine bone projected to the ground and smoothed with a Savitzky-Golay filter.
  - Facing: the hip forward direction projected and smoothed the same way.
  - This works best on long takes that start and end standing, and poorly on pre-cut loops. Manual editing should remain possible.
  - Source: [Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- Fitting the spring to the data ("Fitting Code Driven Displacement," 2022):
  - Fit max velocity, velocity halflife, and facing-angle halflife per movement mode.
  - For a spring from 0 to goal g, peak acceleration occurs at t = 1/y with value v_max = y·e⁻¹·(g − x). So `halflife = damping_to_halflife(2·(v_max/(x_goal − x))·e)`.
  - Use the max velocity sustained over 0.5 s for robustness.
  - Fits: stand-to-run gives about 4.86 m/s and a smoothed max acceleration of about 3.94 m/s², so a halflife of about 0.63 s. A 180° plant-and-turn gives about 154.68 °/s, so a halflife of about 0.60 s.
  - Recommendation: expose spring tuning as max acceleration or max angular velocity rather than halflife.
  - Source: [Fitting Code Driven Displacement](https://theorangeduck.com/page/fitting-code-driven-displacement)
  - Derived comparison (my arithmetic, not stated by Holden): the demo's 0.27 s gameplay halflife is about 2.3× stiffer than the fitted stand-to-run value of 0.63 s. This matches ND's "stiff springs by design" point below.

**Holden: desync handling options, compared in "Code vs Data Driven Displacement" and implemented in controller.cpp**
- Do nothing: the character catches up by matching faster or slower clips, which is insufficient for precise control.
- Sync character to simulation every frame: direct control, but more foot sliding.
- Sync simulation to character: little sliding, but sluggish.
- Blend the two (a compromise; `synchronization_data_factor`, off by default).
- Source: [Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- Adjustment (on by default):
  - `damp_adjustment_exact(g, h, dt) = g·(1 − exp(−ln2·dt/(h + ε)))` applied to the position and rotation difference.
  - Halflives: position 0.1 s, rotation 0.2 s. Rotation is deliberately slower, "since aggressive rotation correction looks bad."
  - Velocity-limited: the adjustment is capped at `0.5 · |character_velocity| · dt` (and similarly for angular velocity), so a stationary character is never slid.
  - Source: [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp); [article](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- Clamping (on by default): `clamping_max_distance = 0.15` m and `clamping_max_angle = 0.5π` (90°). The character is projected back onto the sphere around the simulation position. The article warns that clamping can feel like being "dragged" when pressed against the boundary. — [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp); [article](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- Because the trajectory query is expressed relative to the character entity, not the simulation, any current desync appears directly in the query. The search then prefers clips that close the gap. — [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp)

**Naughty Dog: the Motion Model**
- The root node ("Align" in ND's engine) "drives the capsule and root motion." It is hand-animated for every clip: "Curves must be smooth… Unnecessary lateral motion removed; Align rotation animated in compliance with the Motion Model." — [ND deck slide 8](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- "Motion Model… is our simulation of character motion. We use a critically damped spring to simulate position and velocity and we also simulate turn speed and facing direction." — [ND deck slide 9](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Modes: Stick Input (strafing, non-strafing, cover) for the player; Path Following and Path Transition for NPCs; a Horse mode. — [ND deck slide 10](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Parameters, most of them "directional values":
  - Velocity spring constant, with separate acceleration and deceleration springs. NPCs have "the acceleration spring tighter than the deceleration spring so they can react to player's motion faster but they reach their destination in a more relaxed manner."
  - Max speed in m/s.
  - Turn rate in °/s, added because "without it it's difficult to predict the correct model facing direction."
  - "Translation clamp distance or root motion allowance": NPCs 0.3–0.6 m "to allow more natural motion"; player "None for maximal motion precision."
  - Source: [ND deck slide 11 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
  - Internal inconsistency: the slide 36 notes say AI may offset "up to 0.1-0.4m" and the "Player allows as much 0.01 m offset… Anything less deemed unresponsive." Both describe the player as essentially fully code-driven, but the NPC figures differ (0.3–0.6 m against 0.1–0.4 m). — [ND deck](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Player challenges:
  - "Stiff Motion Model Springs by design (acceleration and deceleration happens faster than in real life)." Starts are "twice faster as human performs on a stage and stops as short as 15-20cm, instead of meters that actor takes."
  - Lack of prediction: "most of the prediction is straight (slightly biased line) that is rapidly changing. It makes selection of turns and corners very hard."
  - Source: [ND deck slide 36 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Procedural pass:
  - Steering rotates the Align so the future Align lands on the trajectory, with the rotation error spread over time.
  - Time scaling is "Normally ± 10 - 15%," with the coefficient on a spring.
  - Clamping is a radial constraint. The basic version has a hard edge; a "Soft clamp variant tracks towards center using a low pass filter."
  - Source: [ND deck slides 41–43](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Blending root and skeleton at different rates: "Motion blend 0.2 - 0.3 sec vs 0.4 - 0.6 sec animation blend," because "blends tend to wash out the character motion therefore trajectory starts falling behind quickly." Blend times can be set per pair of MM sets and per clip pair. — [ND deck slide 45](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)

**Navigation and AI driving the query**
- ND NPCs: "Generated paths are series of straight segments therefore quite different from smooth animated trajectories." The fix is a shorter trajectory window (0.5–1.2 s), "so it has only one shape at a time," with high trajectory and direction weights. — [ND deck slides 38–39](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- ND "Slowing Down At Corners": without it the NPC "whips around corner without slowing down… Weightless feel; Increases foot sliding; The Motion Model deviates from the animation data." The fix:
  - Define at what angles and how much to slow down, using values "sourced from Animation Analyzer."
  - "Collect corners on the path."
  - "Calculate the slow down distance based on the deceleration spring."
  - Source: [ND deck slides 61–62](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- ND NPC entry and scripted move-to: a "Spline trajectory from entry vector to NPC," with the Motion Model speed matched to the entry Align velocity and loop phase matched as described in section 2. For Traversal Action Packs, enter, middle, and exit animations generate the Motion Model trajectory itself. The middle is trimmed or extrapolated to fit the obstacle ("automatically makes the fall last longer"). — [ND deck slides 57–69](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Unreal's Game Animation Sample (GASP) documents "best practices for using motion matching with a capsule driven movement model."
  - A `GenerateTrajectory` function and a Pose History node supply the trajectory.
  - Gameplay predicates read the predicted trajectory: `IsMoving` and `IsStarting` (future velocity greater than current), `IsPivoting` (direction change above a threshold; "45 degree pivots work nicely during strafing"), and `ShouldTurnInPlace` (more than 50° while aiming).
  - Root rotation is steered toward the target through the Motion Matching node.
  - An experimental "Offset Root Bone" lets translation deviate within "a fixed radius" with a configurable halflife.
  - Source: [Epic, Game Animation Sample Project docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)
- Epic's 2025–2026 direction moves the simulation side onto the Mover plugin, which Epic calls the eventual successor of the Character Movement Component. The UE 5.7 sample added "Simple Spring Walking Mode and Smooth Walking Mode." — [Epic tech blog, GASP 5.7 update](https://www.unrealengine.com/tech-blog/explore-the-updates-to-the-game-animation-sample-project-in-ue-5-7) (content seen via search snippet; the page returned 403 to direct fetch)

### Inferences
- The canonical architecture is clear: a gameplay-owned simulation (spring on velocity plus facing, with collision) predicts the trajectory. Animation root motion moves the visible entity. A damped, velocity-limited adjustment plus a hard clamp bounds the error.
- The clamp radius is the main "responsiveness versus quality" knob: about 0.15 m for an action player (Clavet, Holden), about 0 for a precise third-person player (ND), and tenths of a metre for NPCs.
- Byte-Engine already has continuous navigation steering, so an NPC Motion Model can take the steering output as its desired velocity. ND's two NPC lessons transfer directly: limit the trajectory horizon to one path "shape" at a time, and pre-decelerate at path corners using the deceleration spring.
- Separate acceleration and deceleration springs, plus a simulated turn rate, are worth having from the start. ND added turn rate after finding facing prediction poor without it. Holden's fitting method gives data-derived starting values.
- Expressing the trajectory query relative to the animated entity (Holden) means the search corrects drift on its own. This is cheap and needs no extra feature.

### Gaps
- No primary source gives *For Honor*'s spring halflives or the exact form of its 15 cm clamp, for example whether it is soft or hard.
- ND does not publish numeric spring constants, max speeds, or turn rates.
- Epic's docs do not give the default Offset Root Bone radius or halflife, or the Character Trajectory component's default prediction and history sampling.

---

## 4. Production write-ups and the industry's development (2016 to 2026)

### Takeaway
After *For Honor* (2017), motion matching spread across AAA studios in three waves:
- **2018–2021:** EA *UFC 3* "Real Player Motion," EA FIFA and Madden interactions, Remedy *Control*, and Naughty Dog's *The Last of Us Part II* (2020), the most detailed public production account.
- **2020:** Ubisoft La Forge (Holden et al.) formalized the algorithm, with learned variants as a successor; out of scope here.
- **2024–2026:** Epic made Pose Search production-ready in UE 5.4 (2024) and kept extending it through UE 5.8 with choosers, Mover, and multi-character interactions.

Production practice moved away from the 2016 pitch of dumping unstructured dance-card mocap into a database. ND instead used structured, cut, heavily polished medium and short takes, controlled clip "heads and tails," cost-bias groups, and tooling for debugging and weights. Holdouts such as Bobby Anguelov argued the technique is overhyped.

### Cited Findings

**Naughty Dog, *The Last of Us Part II* (GDC 2021, Michal Mach and Maksym Zhuravlov)**
- Motivations:
  - Quality for "a very grounded game."
  - Scale: "over a dozen of unique gameplay character archetypes."
  - Stability: "Unlike state machine based locomotion, Motion Matching cannot get stuck on undefined transition… We might see occasional visual glitches, but they won't cause progression stopping bugs."
  - It was "easy to try since the foundation of the system is actually pretty simple."
  - Source: [ND deck slide 5 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Data:
  - "We don't mocap unstructured data or long dance cards… Instead, we direct medium to short takes."
  - Player premise: "Animations shouldn't 'shred' on straight inputs… if you just press stick forward, hold and then release – you should see exactly 3 animations, 4 at worst but never 5."
  - Full starts of 25–40 frames depending on the locomotion style.
  - Source: [ND deck slides 15–16 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Early discoveries:
  - "Minimum-enough coverage is good."
  - "Animation 'heads' and 'tails' should be of specific controlled length." If start-left has a 90-frame tail and start-right only 15, one gets stuck and the other swallowed.
  - "It is still vital to bracket animations with core poses."
  - Transitions are split per file and named by convention, with a set-authoring wizard and farm processing.
  - A `loopsInsert` tool overlays polished loops, held to a sync frame, on transition tails, so clips end with "exact pose cost match."
  - Source: [ND deck slides 17–19 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Scale:
  - Player: 2,627 clips, 2 h 17 min (Ellie and Abby combined).
  - NPCs: 6,050 clips, 6 h 34 min in total (human enemies 2,387 clips / 2 h 24 min; buddies 678 / 43 min; infected 768 / 43 min; animals 431 / 42 min).
  - Male and female enemies share "androgenous move sets."
  - Source: [ND deck slides 20, 23, 26–27](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Performance (context only): "Index search ~0.1ms; Complete update 0.5‒1.0 ms… The old locomotion system took 2‒3 ms a frame." — [ND deck slide 83](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Pros:
  - "Very high animation connectivity."
  - "Transitioning from 'canned' animations into a locomotion set comes for free." Door animations needed only "abort frames."
  - "Higher stability."
  - "Faster in runtime."
  - Cons:
    - "Requires technically minded animators."
    - "Rather memory hungry."
    - "High upfront cost."
    - "Motion Model creation can easily become a bottleneck."
    - It "Relies on large MoCap stage access bandwidth."
    - Left- and right-foot versions of every move strain stunt performers.
  - Source: [ND deck slides 52, 84–85](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Scope: Mach's reel says MM was used "mainly for character locomotion but also traversal, melee and player/npc cover system," and that he "spearheaded" it with John Bellomy. — [Mach reel, Vimeo](https://vimeo.com/433024043)

**Ubisoft**
- *For Honor* is the first shipped title to use MM for player control. See sections 1–3 and [Game Anim](https://www.gameanim.com/?p=13538).
- Ubisoft Toronto's 2016 prototype listed its failures:
  - "Editing can be Labour Intensive… the system gets you to 70% real quick… but the last 30% can be tough."
  - "Data Heavy."
  - "Restricted to Human-like Rigs… All animation full body."
  - Designers altering data "aggressively" broke results.
  - Mirroring failed.
  - "Only used for Locomotion so far."
  - Source: [Zadziuk slides/notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- Ubisoft La Forge's 2020 LMM paper describes basic MM as "our own state-of-the-art implementation used in several AAA game productions." It also says MM "has quickly been adopted by many studios due to its simplicity, flexibility, controllability, and the quality of the motion it produces," and cites Büttner 2019, Clavet 2016, Harrower 2018, and Zinno 2019. — [LMM paper](https://theorangeduck.com/media/uploads/other_stuff/Learned_Motion_Matching.pdf)
- Clavet later presented "Ragdoll Motion Matching" at a GDC Machine Learning Summit. — [GDC Vault](https://www.gdcvault.com/play/1026712/Machine-Learning-Summit-Ragdoll-Motion). I did not confirm the year from the page.

**EA**
- GDC 2018, Geoff Harrower, "Real Player Motion Tech in 'EA Sports UFC 3'." The team "rebuilt its gameplay and animation systems around RPM," with path generation and manipulation for coherence with "the RPM database query," an RPM Event System for timing constraints, and layered techniques for pose variety. — [GDC Vault abstract](https://gdcvault.com/play/1025228/Real-Player-Motion-Tech-in)
- GDC 2019, Fabio Zinno, "ML Tutorial Day: From Motion Matching to Motion Synthesis, and All the Hurdles In Between." The talk moves from MM toward neural synthesis. — [GDC Vault](https://gdcvault.com/play/1025890/ML-Tutorial-Day-From-Motion)
- GDC 2021 Animation Summit, Henry Allen, "Environmental and Motion Matched Interactions; 'Madden', 'FIFA' and Beyond!" It covers two ways to combine MM with multi-character interaction without losing player control: single-character MM for transitions into multi-character animation, and synchronized multi-character MM. It is a follow-up to 2017's "EA Presents: N-teract." — [GDC Vault](https://www.gdcvault.com/play/1027465/Animation-Summit-Environmental-and-Motion)
- GDC 2023 Animation Summit, JC Delannoy, "Motion Matching at EA: Five Years Later." Per the abstract, EA began integrating MM "over five years ago" and the talk covers how usage evolved. It focuses on concepts for animators and designers, not core algorithms. — [GDC Vault](https://www.gdcvault.com/play/1028821/Animation-Summit-Motion-Matching-at)
- *FIFA 22* HyperMotion moved toward ML ("Full-Match Mocap Driving Machine Learning Technology"), which is outside classic scope. — [GDC Vault](https://gdcvault.com/play/1027746/Animation-Summit-FIFA-22-s)

**Remedy**
- GDC 2021 Animation Summit, "Take 'CONTROL' of Animation" (Ville Ruusutie, Ilkka Kuusela). Remedy built its own animation system after its middleware "was taken off the market." Topics include "motion matching ideology," debugging motion matching, and "animation driven motion matching." — [GDC Vault](https://gdcvault.com/play/1026988/Animation-Summit-Take-CONTROL-of); [alanwake.info](https://www.alanwake.info/2021/07/16th-july-2021-upcoming-gdc-2021-talk.html)

**Epic / Unreal Engine (design philosophy and timeline)**
- UE 5.4 (2024) moved Motion Matching from experimental to production-ready. — [CG Press, UE 5.4 release](https://cgpress.org/archives/unreal-engine-5-4-released.html)
- The Game Animation Sample shipped with UE 5.4 with over 500 animations. Epic calls it a sandbox for "understanding Motion Matching best practices for AAA teams," focused "mainly on animation features, rather than gameplay systems." — [Epic GASP docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine); [Epic blog](https://www.unrealengine.com/blog/game-animation-sample?lang=en-US)
- Epic's philosophy, as reflected in the docs:
  - Keep gameplay state explicit and use MM for pose selection inside it. "Choosers… help us control motion matching by filtering down which databases can be selected from based on gameplay context." For example, "we only search from the walk databases when the character is walking."
  - The blend stack interrupt mode, "By default, we do not interrupt," is forced on core state changes.
  - Blend time depends on the current and previous states.
  - Source: [Epic GASP docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)
  - This is a hybrid of state logic and MM, close to Zadziuk's 2016 "Hybrid System: Motion Matching as Animation Node" ([Zadziuk slides](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)) and unlike ND's "stateless" goal.
- Talks:
  - GDC 2024 "Unreal Engine 5.4 Animation Deep Dive" (sponsored) included "how we reinvented Fortnite Battle Royale's locomotion with Motion Matching." It was not recorded for the Vault. — [GDC 2024 schedule](https://schedule.gdconf.com/session/unreal-engine-54-animation-deep-dive-presented-by-epic-games/904103)
  - Unreal Fest 2024 "Motion Matching & Game Animation in UE 5.4." — [YouTube](https://www.youtube.com/watch?v=tNw9lD2PW3U), listed by [Game Anim](https://www.gameanim.com/?p=124607)
- UE 5.7 sample: Mover-based character with new spring walking modes. UE 5.8 sample: "multi-character motion matching" through "Pose Search Interaction Assets," a reworked locomotion Chooser with an improved "Pose Match Column" where "each asset using a Pose Match Column is able to define the database it belongs in," and a ragdoll pawn. — [Epic tech blog, GASP 5.8](https://www.unrealengine.com/tech-blog/download-the-latest-game-animation-sample-project-now-updated-for-ue-5-8); [The Rookies summary](https://www.therookies.co/blog/headlines/unreal-engines-game-animation-sample-project). Epic pages returned 403 to direct fetch, so this content comes from search snippets.
- Epic's current docs are labelled "Unreal Engine 5.8 Documentation." Database search modes are Brute Force, PCAKDTree, and VPTree (experimental). — [Epic docs](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)

**Dissent: Bobby Anguelov**
- May 2021, on X: "Unpopular game animation opinions: Anim-driven locomotion is a mistake. Motion-matching is a solution looking for a problem, also has massive issues people don't talk about or understand. Gameplay is king, animation HAS to be the one to compromise for gameplay." — [X post](https://x.com/Bobby_Anguelov/status/1395375951639453700)
- June 2022: he argued against auto-generating root motion, saying it failed on every project he worked on, and favoured investing in authoring tools. — [X post](https://twitter.com/Bobby_Anguelov/status/1539982427111702530) (content seen via search summary)
- His Esoterica engine advertises an "AAA quality high performance animation system." I found no source tying Esoterica to motion matching. — [GameFromScratch](https://gamefromscratch.com/esoterica-game-engine/)

**2023–2026 talks and research found**
- GDC 2023: EA "Five Years Later" (above).
- GDC 2024: Tencent "MotorNerve," which combines MM with learned MM for locomotion (neural, out of scope). — [GDC 2024 schedule](https://schedule.gdconf.com/session/motornervea-character-animation-system-using-machine-learning-presented-by-tencent-games/903481)
- GDC 2025: an Epic-led UE 5.5 motion matching workshop (Luis Cataldi), documented by an attendee blog titled "gdc2025". Whether it was an official GDC session is not verified. — [Michelle Molina blog](https://www.michellemolina3d.com/blog/gdc2025-motion-matching-workshop-documentation)
- SIGGRAPH Asia 2025: "Environment-aware Motion Matching" (Ponton, Andrews, Andujar, Pelechano). It adds body-shape features and a dynamic collision penalty to the search, coupling pose and trajectory for crowds. It appears to be non-neural, but I did not confirm from the full text. — [arXiv 2510.22632](https://arxiv.org/abs/2510.22632)

### Inferences
- By 2026 the field has two production philosophies:
  - **ND, "stateless."** One MM set per locomotion style, soft cost-bias groups and layers, and transition handover controlled by clip cut length.
  - **Epic, "stateful hybrid."** Gameplay predicates and Choosers pick databases, and MM picks the frame. Both are proven.
- The Epic model fits engines that want explicit gameplay state and debuggability. The ND model gives better connectivity but puts the burden on content authoring and Motion Model tuning.
- The 2016 promise of "plug in raw mocap" did not survive production. Every detailed later source (ND, Epic's structured databases, Zadziuk's own "RIGHT data, not ALL the data" note) relies on curated, cut, polished, convention-named data.

### Gaps
- I found no public technical talk on motion matching in *Ghost Recon* or *Assassin's Creed*. Whether those titles use classic MM, and how, is unverified.
- GDC Vault was in maintenance during this research (it redirected to gdconf.com/gdc-vault-maintenance), so only abstracts were available for EA and Remedy talks. Their technical content is not captured.
- I found no GDC 2025 or GDC 2026 Vault session specifically on classic motion matching. Absence from search results does not prove none exists.
- I found no talk, blog series, or detailed write-up by Bobby Anguelov on motion matching beyond social posts.

---

## 5. Known pitfalls and their production fixes

### Takeaway
The recurring failure modes are:
- **Shredding:** too-frequent switching between near-duplicate clips. It washes out the performance and causes flicker.
- **Swallowed or stuck transitions.**
- **Confusion between moves with similar trajectories:** stops versus quick-plants, starts versus corners.
- **Foot sliding** from frequent blends and from keeping up with stiff gameplay springs.
- **Unresponsiveness** when animation drives displacement.
- **Coverage gaps and costly tuning.**

The fixes are consistent across sources:
- continuation bias, neighbour and recent-pick exclusion, and search throttling;
- past-trajectory samples and "moving" or "loop" bias groups;
- controlled clip head and tail lengths;
- curated minimum-enough coverage via dance cards or structured takes;
- toe locking, foot IK, and gentle timescale or steering;
- a bounded simulation-to-entity clamp;
- grouped, normalized weights plus strong debug visualization.

### Cited Findings

**Shredding and flicker**
- ND: "Animation shredding: Animations would get dropped very frequently and another very similar animations would be played… the character animation gets a really washed out look." Also: "Continuity of transforms is not the same thing as continuity of the character performance." — [ND deck slides 6, 16 notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- ND causes: hard springs invalidate starts by velocity and stops by position and pose. A straight-line prediction changes rapidly. With no root offset allowed, aggressive dragging invalidates clips faster. "Every time switch happens – blend has to be applied… [it] also washes out some details." — [ND deck slide 36 notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- ND fixes:
  - Natural (continuation) bias, but not so high that clips feel "sticky."
  - A Core Loops group for steady input.
  - Low current-velocity weight.
  - High pose weight backed by pose-matched content.
  - "Minimum enough coverage," because too many similar animations make "MM… use different pieces of similar transitions to create a single much faster transition."
  - Source: [ND deck slides 17, 32–33, 37 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Holden: ignore candidates within 20 frames of the current frame, search on a 0.1 s timer rather than every frame, and switch only on a strictly lower cost than the current frame. — [database.h](https://github.com/orangeduck/Motion-Matching/blob/main/database.h); [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp)
- UE: Continuing Pose Cost Bias, Pose Jump Threshold Time, Pose Reselect History, Search Throttle Time, and Block Transition notifies. — [Epic docs](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)
- Clavet treated frequent switching as acceptable ("Switch multiple times per second") and relied on blending and toe locks. — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf). This is 2016 practice; ND's later premise of 3–4 clips for a straight input is stricter.

**Swallowed or stuck transitions**
- ND: "Transitions tend to either get 'stuck' or get completely 'swallowed' depending on settings." Starts were swallowed and stops "abrupting to idles." The fix is controlled "heads and tails" lengths, so transitions drop out when they run out of future samples, plus bracketing with core poses and loop insertion. — [ND deck slides 6, 16–17, 37 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Zadziuk: "For each 'move' to be read by the system we would need to add about a second either side so that the paths of the animation could be seen by the prediction model." — [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)

**Confusing moves with similar trajectories**
- ND: "Quickplants play instead of stops. While they share similar path and velocity, the performance of the quickplant is very different from the stop." The fixes are the Moving Group bias, which is sprung on Motion Model speed, and a compulsory past-trajectory sample at 0.4–0.7 s. — [ND deck slides 6, 34, 37 + notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Zadziuk: "We couldn't always be sure it would pick the move we 'wanted'… providing happy accidents but equally un-predicatble results." — [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- Epic's alternative is explicit gameplay predicates (IsStarting, IsPivoting, IsMoving) and Choosers that pick pivot, start, or stop databases. Pivot databases are excluded from start searches "so a pivot isn't interrupted." — [Epic GASP docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine)

**Foot sliding**
- Clavet: "Sliding Prevention." Sliding comes from "Blending very often" and "Keeping up with gameplay." The fix: "Lock the toe when it doesn't move too much in the main animation." Slope warping pulls the hips down without hyper-extending the knee, and "Slope anims are automatically chosen by 3D trajectory matching; IK only compensate[s] for what is missing." — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf)
- Game Developer's summary of Clavet's nucl.ai 2016 talk:
  - Use simple foot-velocity and ground-proximity heuristics for foot IK.
  - "Don't lock a foot that sits far from its animated pose."
  - Prefer sliding to time-scaling.
  - Source: [Game Developer](https://www.gamedeveloper.com/programming/most-inspiring-game-animation-tech-talks-of-2016)
- ND: foot-plant IK runs "all the time when locomoting," pinning the foot below a velocity threshold with a Jacobian IK solver. NPCs slow down at corners because whipping round them "Increases foot sliding." — [ND deck slides 44, 61](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Holden: foot locking and IK with an unlock radius of 0.2 m, foot height 0.02 m, toe length 0.15 m, and an IK blend halflife of 0.1 s. A velocity-limited adjustment means a stationary character is never slid. — [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp); [Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement). Transition and IK details are covered by other notes.

**Unresponsiveness versus quality**
- Clavet: predictability over raw responsiveness, with a code-driven simulation and a 15 cm clamp. — [Clavet slides](https://media.gdcvault.com/gdc2016/Presentations/Clavet_Simon_MotionMatching.pdf); [Game Anim](https://www.gameanim.com/?p=13538)
- Holden: syncing the simulation to the character produces "sluggish response," and syncing the character to the simulation produces sliding. The default is adjustment plus a clamp. — [Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- ND player: effectively no offset is allowed, so responsiveness comes from stiff springs. The cost is borne by content: high pose weight, curated starts and stops, and loop insertion. — [ND deck slides 11, 36–37](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Holden forces a new search when a rapid input change ends, rather than waiting for the 0.1 s timer. — [controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/main/controller.cpp)

**Coverage of starts, stops, turns, pivots, strafes**
- The *For Honor* dance card had seven categories:
  1. Walks and runs
  2. Small repositions
  3. Starts and stops
  4. Circles
  5. Plant-and-turn at 45°, 90°, 135°, and 180°
  6. Square strafing with 90° plants
  7. Strafe plants for 180° shifts

  Clavet also noted the value of capturing fatigue late in a shoot. — [Game Anim](https://www.gameanim.com/?p=13538)
- Ubisoft Toronto split its dance cards into separate files: starts and stops, plant and turns, circles, acceleration and deceleration (including long cycles and speed transitions), and "the key for me… Snakes." Capture was repeated for walk, jog, and run. "Capture the minimum amount of moves to create maximum coverage." — [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- ND rejected long dance cards in favour of medium and short directed takes, with left- and right-foot variants (the wrong foot penalized) and mirroring for some sets. — [ND deck slides 15, 23](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Holden: mirror the data for coverage and speed it up "by about 10%" for more maximum speed. — [Code vs Data Driven Displacement](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- Player prediction is always extrapolated, so moves like "go forward slow down and turn 130" are pointless data: "it doesn't do any useful work, just fights other animations eats memory." — [ND deck slide 17 notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)

**Data gaps and data quality**
- Zadziuk: "the system performed better if we fed it the RIGHT data as opposed to ALL the data." Also: "sometimes removing data from a file even if we felt it wasn't being used could cause issues." — [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf)
- ND: "Data consistency is key." The Align trajectory was semi-automated but still needed touch-up "because Maya's auto tangents cause velocity spikes." An "Animation Analyzer" tool lets the team "marry the animation data and motion model." — [ND deck slides 24–25](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- ND on unstructured data in production: with 15 candidate clips for "start forward," a director's note has no clear target. "When you animate the pose cost changes, so animation you are fixing even can stop from being picked." — [ND deck slide 16 notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)

**Parameter tuning and debugging**
- ND: "Weights Balancing… can be intimidating." The fixes are normalization to σ = 1, grouped weights with master multipliers, and in-game debug draw:
  - a list of the best candidates with their costs and a cost breakdown;
  - the current pose as a blue stick figure and the selected candidate as a red one;
  - the animation trajectory in red and the Motion Model trajectory in blue.
  - Source: [ND deck slides 28–30, 48](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)
- Holden: features must be regenerated when weights change, and spring parameters can be fitted from data. — [README](https://github.com/orangeduck/Motion-Matching); [Fitting Code Driven Displacement](https://theorangeduck.com/page/fitting-code-driven-displacement)
- Epic ships dedicated Pose Search debugging tools and a Rewind Debugger view for Motion Matching nodes. — [Epic MM debugging docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-debugging-in-unreal-engine)
- Effort curve:
  - Zadziuk: "70% real quick… the last 30% can be tough."
  - ND: "you get really fast to 60% of the quality, but getting to 100% becomes exponentially harder."
  - Sources: [Zadziuk notes](https://media.gdcvault.com/gdc2016/Presentations/Zadziuk_Kristjan_MotionMatchingFutureOfGamesAnimation.pdf); [ND deck slide 18 notes](https://s3.amazonaws.com/nd.images/research/tlou2/motion_matching_in_tlou2_nd_final.pptx)

### Inferences
- For Byte-Engine, the anti-shredding toolkit should be part of the core search contract, not something bolted on later:
  - The current frame is the baseline the search must beat.
  - A continuation bias.
  - Exclusion of a window around the current frame.
  - A recently-selected history.
  - Unpickable tail frames that lack full future samples.
  - A throttle with a forced re-search when input settles.

  Each is a few lines in Holden's or UE's design. They interact, so they should be tunable per database.
- Soft bias groups (ND's Moving and Loops groups, with the group enable signal sprung) handle similar-trajectory confusion with less state than Epic's predicate-and-Chooser approach. Debug visualization is the deciding cost either way: both ND and Epic built candidate lists with cost breakdowns and overlays of the model and animation trajectories.
- The authoring pipeline matters as much as runtime code. It needs a convention for clip head and tail length, core-pose bracketing, a root (Align/simulation bone) curve fitted to the Motion Model, and per-move left- and right-foot variants or mirroring. Without them, runtime tuning cannot reach the final quality.

### Gaps
- No source gives quantitative before-and-after metrics for these fixes, such as switch frequency or slide distance. Evidence is qualitative or from video.
- No source gives default numeric values for UE's continuation bias, Pose Jump Threshold, or Pose Reselect History.
- No source gives ND's absolute bias magnitudes beyond the 1000-cost layer penalty.
