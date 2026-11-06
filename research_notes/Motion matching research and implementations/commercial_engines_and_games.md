# Commercial motion matching: engines, middleware, shipped games, and production experience (state as of October 2026)

Scope note: these notes cover commercial and production use. Open-source repositories are out of scope except where a vendor product became open source. Unreal's Pose Search is covered as a product, not as source code. Several primary sources were unreachable during research: the GDC Vault was in maintenance (redirected to gdconf.com/gdc-vault-maintenance), and unrealengine.com blog pages and some news sites returned HTTP 403. Where a claim comes only from a search-engine excerpt of a page I could not open, the finding says so.

## 1. Engine-level offerings: Unreal Pose Search, Unity, in-house engines, and CD Projekt Red's Witcher 4 work

### Takeaway
Unreal Engine is the only general-purpose commercial engine that ships a first-party, production-ready motion matching system. Pose Search went from Experimental to Production-Ready in UE 5.4 (April 2024) after Fortnite adopted it in December 2023. Since then, Epic has spent each release joining it to the Chooser, to Mover, and, in UE 5.8 (June 2026), to experimental multi-character "interaction" searches, the feature CD Projekt Red showed in its June 2025 Witcher 4 demo. Unity has no built-in motion matching: it suspended Kinematica in 2021 and, in May 2025, said motion matching would not be in the first release of its new animation system. Among in-house engines, public confirmation exists for EA (Frostbite-era R&D group), Naughty Dog, Remedy Northlight, IO Interactive Glacier, and the open-source O3DE. For Ubisoft Anvil and Snowdrop, Guerrilla Decima, and Insomniac I found no primary source.

### Cited Findings

**Unreal Engine: status timeline**
- By May 2023, Epic's motion matching implementation already existed in UE5 as a plugin named "PoseSearch". A May 16, 2023 community deep dive describes it. — [jiahaoli.org blog, 2023-05-16](https://jiahaoli.org/blog/2023/05/16/motion-matching-in-unreal-engine-53-the-plugin-pose-search-in-unreal-engine/)
- UE 5.4 (announced at GDC in March 2024, released April 2024): Epic's release post says "Motion Matching, previously introduced as an Experimental feature, is now Production-Ready". The post says Epic tested it in Fortnite Battle Royale on all 100 characters plus NPCs, on platforms from mobile to console. The same release added Choosers, a tool that uses game context to drive animation selection. I took this from the search excerpt because the page returned 403. — [Unreal Engine 5.4 announcement (Epic, April 2024)](https://www.unrealengine.com/blog/unreal-engine-5-4-is-now-available); syndicated in [Game World Observer, 2024-03-20](https://gameworldobserver.com/2024/03/20/unreal-engine-5-4-details-uefn-ugc-tools-egs-mobile)
- A GDC 2024 write-up says Fortnite had used Motion Matching on all characters and all platforms since Chapter 5 launched in December 2023. — [Jon Peddie Research, "Epic announcements at GDC 2024" (March 2024)](https://www.jonpeddie.com/news/epic-announcements-at-gdc-2024/)
- At GDC 2024, Epic gave a session called "Unreal Engine 5.4: Animation Deep Dive". It covered how Fortnite Battle Royale's locomotion was rebuilt with Motion Matching. — [YouTube, Unreal Engine (2024)](https://www.youtube.com/watch?v=u9Z8CK561_Y); [Epic GDC 2024 events page](https://www.unrealengine.com/en-US/events/gdc2024)
- Epic's support knowledge base says its GDC Motion Matching presentation described a custom Fortnite node called "Copy Motion" for layering animation on top of the motion matching output. This is from a search excerpt; the page failed TLS verification when fetched. — [Epic support: "Using Copy Motion node for layering with Motion Matching"](https://eoshelp.epicgames.com/s/article/Using-Copy-Motion-node-for-layering-with-Motion-Matching?language=en_US)
- Game Animation Sample Project (GASP), released June 12, 2024 for UE 5.4: more than 500 "AAA-quality" animations, a full character and Animation Blueprint, and a motion matching toolset. The data is the locomotion and traversal dataset from the GDC 2024 keynote demo. — [GameFromScratch, 2024-06-12](https://gamefromscratch.com/epic-games-release-500-free-aaa-quality-game-animations/); [Epic blog "Get over 500 free animations"](https://www.unrealengine.com/en-US/blog/game-animation-sample)
- In GASP, animation selection is driven mostly by Choosers and Proxy Tables over several Motion Matching databases. This comes from a third-party summary. — [uhiyama-lab notes](https://uhiyama-lab.com/en/notes/ue/motion-matching-game-animation-sample/)
- UE 5.7 (November 2025): Epic's GASP update for 5.7 centered on the Mover plugin, which was still Experimental. It also added an Experimental "Pose Search Column" in Choosers that brings Choosers and Motion Matching closer together. This is from a search excerpt; the page returned 403. — [Epic tech blog "Explore the updates to the Game Animation Sample Project in UE 5.7"](https://www.unrealengine.com/tech-blog/explore-the-updates-to-the-game-animation-sample-project-in-ue-5-7); release month from [AlternativeTo news, 2025-11](https://alternativeto.net/news/2025/11/unreal-engine-5-7-adds-procedural-content-generation-nanite-foliage-and-metahuman-updates)
- UE 5.8 is listed as the current stable release, dated June 17, 2026. — [Wikipedia: Unreal Engine 5](https://en.wikipedia.org/wiki/Unreal_Engine_5)
- UE 5.8 release notes (Animation > Gameplay), as quoted in a search excerpt:
  - Pose Search databases created for Chooser columns now default to BruteForce "since KD Tree usually fails with external filtering".
  - New Experimental `UPoseSearchInteractionLibrary::MotionMatchMulti`, an API that performs "multi character motion matching searches without any frame delay".
  - New `OverridePoseHistoryFromOwningMesh`, which overrides pose history with the mesh pose, for example to motion match physics poses.
  - New `UAnimNotifyState_PoseSearchSamplingRange`, which marks the animation segments a database includes.
  - A project setting that caps editor preview actors (default 50).
  - Disabled database animations are now stripped at cook.
  - Separately, "ChaosMover adds trajectory prediction for motion-matching workflows", while Mover itself is still working toward leaving Experimental.
  - Sources: [UE 5.8 Release Notes (Epic, 2026)](https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5-8-release-notes?lang=en-US); the ChaosMover line was confirmed by a direct fetch of the same page.
- GASP for UE 5.8 (mid-2026):
  - New "Pose Search Interaction Assets" give a simple setup for multi-character motion-matched animations, with motion-warp tooling.
  - A ragdoll pawn performs multi-character motion-matched interactions and motion-matched get-ups after ragdoll.
  - The Mover Pawn's state-machine-driven blend stack was reworked into a Chooser with an improved Pose Match Column that can hold several Pose Search Databases.
  - Sources: [Epic tech blog, GASP updated for UE 5.8](https://www.unrealengine.com/tech-blog/download-the-latest-game-animation-sample-project-now-updated-for-ue-5-8) (search excerpt; 403 on fetch); [The Rookies summary](https://www.therookies.co/blog/headlines/unreal-engines-game-animation-sample-project)
- The multi-character interaction API is labeled "Experimental, this feature might be removed without warning, not for production use". Interacting actors form an "interaction island", and the actor with the highest tick priority becomes the main actor. — [Epic API docs: Motion Match Multi](https://dev.epicgames.com/documentation/unreal-engine/BlueprintAPI/Animation/PoseSearch/MotionMatchMulti?lang=en-US); [FPoseSearchInteractionAvailability](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/FPoseSearchInteractionAvailabili-?lang=en-US)

**Unreal Engine: product structure, from the UE 5.8 documentation (2026)**
- Overall structure:
  - Motion Matching is an Animation Blueprint node that replaces State Machines or Blendspaces. It needs the Pose Search plugin.
  - A Pose Search Schema holds configuration and query settings. New schemas come with a Trajectory channel and a Pose channel.
  - A Pose Search Database holds Animation Sequences (the default), Animation Composites, and Blendspaces. Montages are not supported in Animation Blueprints.
  - Source: [Epic docs: Motion Matching in Unreal Engine (UE 5.8)](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- Channels:
  - Standard: Group, Heading, Padding, Phase, Pose, Position, Sampling Time, Trajectory, and Velocity.
  - Experimental ("should not be relied upon in production"): Crashing Legs and Permutation Time.
  - "Motion Matched Stitching" is experimental. — same source
- Inputs: trajectory comes from the Character Trajectory component, and a Pose History node collects the sampled bones. Optional Normalization Sets group databases and can be swapped at runtime. A chooser can set the active database at runtime. — same source
- Search modes and costs:
  - Brute Force evaluates every indexed pose.
  - PCAKDTree projects poses into PCA space and uses a KD-tree. Its results are approximate, so a subset of candidates gets a full cost evaluation.
  - VPTree is experimental.
  - Memory and performance knobs: Number of Principal Components, KDTree Max Leaf Size, KNN neighbors, Pose Pruning Similarity Threshold (merges similar poses), Sample Rate, channel and sample counts, and 16-byte "Add Data Padding".
  - The blend stack has Max Active Blends (0 disables it) and Store Blended Pose. Discarding overflow blends can cause popping.
  - Source: same source
- Debugging: with the Animation Insights plugin, the Rewind Debugger shows a Pose Search track.
  - The details panel has Active Pose, Continuing Pose, and Pose Candidates tabs.
  - Each row shows Database, Asset, Cost (with a per-channel breakdown), Trajectory Total, Pose Total, Bias, Frame, Mirror, Loop, and Flags such as `PoseReselectHistory`.
  - A green-to-red heat map colors the costs, and selected candidates draw their skeleton and ideal trajectory in the viewport.
  - Source: [Epic docs: Motion Matching Debugging in Unreal Engine](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-debugging-in-unreal-engine)

**Fortnite (Epic, in-engine production user)**
- Chapter 5 Season 1 (December 2023): "players on all platforms will experience Motion Matching and Procedural Layering", which improved walk-to-run transitions, direction changes, and weapon use. Physics-based animation was limited to PS5, Xbox Series X, and PC, and affected gliders most. — [Fortnite.com Chapter 5 Season 1 news (Dec 2023)](https://www.fortnite.com/news/join-the-fight-in-fortnite-battle-royale-chapter-5-season-1-underground?lang=en-US) (search excerpt; 403 on fetch); [80.lv, 2023-12-04](https://80.lv/articles/enjoy-improved-animations-in-fortnite-chapter-5)
- Animation Production Lead Colin Friday said the overhaul took about a year. — [80.lv, 2023-12-04](https://80.lv/articles/enjoy-improved-animations-in-fortnite-chapter-5)
- The new movement drew player backlash over slower walking, crouching, and strafing. Epic said it was listening, and patch v28.01.01 sped up crouching and running, with the animations changed to match. — [GameSpot (Dec 2023)](https://www.gamespot.com/articles/yes-fortnite-feels-different-with-chapter-5-and-epic-is-listening-to-feedback/1100-6519678/); [Sportskeeda (Dec 2023)](https://sportskeeda.com/fortnite/fortnite-movement-increased-weeks-epic-games-confirms)

**CD Projekt Red, The Witcher 4 Unreal Engine 5 tech demo (June 3, 2025)**
- The demo was shown at the State of Unreal keynote at Unreal Fest Orlando on June 3, 2025. CDPR presented "Multi-Character Motion Matching" in UE5, which keeps Ciri and her horse Kelpie "perfectly synchronized" when mounting from any angle and speed. — [Game Informer, 2025-06-03](https://gameinformer.com/2025/06/03/get-a-new-look-at-the-witcher-iv-in-unreal-engine-technical-showcase); [Windows Central (June 2025)](https://www.windowscentral.com/gaming/the-witcher-4-ue5-tech-demo-state-of-unreal-2025); [TechRadar (June 2025)](https://www.techradar.com/gaming/five-things-we-learned-from-the-witcher-4s-technical-demo)
- Epic's Wyeth Johnson said, "We also support Root Motion movement on Kelpie". Kelpie's muscles used the Chaos Flesh solver and ML deformers. The demo ran on a base PS5 at 60 fps with ray tracing. CDPR said it was "a tech demo ... but not The Witcher 4 itself". It is CDPR's first project on UE5 instead of its own REDengine. — [GamesRadar (June 2025)](https://www.gamesradar.com/games/the-witcher/the-witcher-4-sees-red-dead-redemption-2s-horse-anatomy-and-submits-realistic-muscles-moving-and-stretching-under-the-skin-of-ciris-horse-kelpie/); [HotHardware (June 2025)](https://hothardware.com/news/witcher-4-unreal-engine-5-6-demo); [Tribune (June 2025)](https://tribune.com.pk/story/2549319/witcher-4-tech-demo-shows-unreal-engine-5s-latest-features)

**Unity**
- Kinematica, Unity's motion matching package, stopped at version 0.8 (experimental).
  - On March 12, 2021, Unity staff (FrancoisFournel) wrote that "all development concerning Kinematica is temporarily suspended and will not resume before 2022". Unity was moving effort to a well-integrated animation suite.
  - On March 7, 2023, staff said no update was planned. Earlier Kinematica releases had been incompatible with Mecanim, had no animation compression, and integrated poorly with other packages, so the team focused on DOTS Animation instead.
  - Source: [Unity Discussions, "Unite Now – Kinematica" thread, p.9](https://discussions.unity.com/t/unite-now-kinematica-democratizing-motion-matching-for-character-animation/788112?page=9)
  - A user noted the last package update ("0.8.0 Experimental") came on August 3, 2020. — [Unity Discussions, "What happened to Kinematica?"](https://forum.unity.com/threads/what-happened-to-kinematica.1200490/)
- On March 19, 2025 (the GDC 2025 roadmap), Unity's Animation PM Eric Dziurzynski announced a new ECS-based animation system for "the Unity 6 lifecycle". It is multi-core, works with Entities and GameObjects, and ships as a package that coexists with Mecanim. He gave no date. — [Unity Discussions, Animation Status Update Q1 2025](https://discussions.unity.com/t/animation-status-update-q1-2025-gdc-roadmap/1614718)
- On May 5, 2025, Dziurzynski wrote: "No, motion matching won't be a part of our initial experimental release". He said Unity would consider it "longer term". — [same thread, p.3](https://discussions.unity.com/t/animation-status-update-q1-2025-gdc-roadmap/1614718?page=3)
- A search excerpt of a later page in the same thread says Unity is "keeping Kinematica alive in-house" to test against the new system. I did not open that page. — [same thread, p.4](https://discussions.unity.com/t/animation-status-update-q1-2025-gdc-roadmap/1614718?page=4)
- A user post in the Unite 2025 roadmap thread (late 2025) claims the new animation system was paused and might land around 2028. This is user speculation, not a Unity statement. — [Unity Discussions, Unite 2025 roadmap](https://discussions.unity.com/t/the-unity-engine-roadmap-unite-2025/1696495)

**In-house and other engines**
- EA: JC Delannoy (Senior Software Engineer, EA applied R&D group for animation) gave "Motion Matching at EA: 5 Years Later" at GDC 2023. The talk covers "productizing" motion matching across EA. Section 4 summarizes the slide content. — [GDC 2023 slides PDF](https://media.gdcvault.com/gdc2023/Slides/Motion+Matching+at+EA_Delannoy_JC.pdf); [GDC Vault listing](https://www.gdcvault.com/play/1028821/Animation-Summit-Motion-Matching-at)
- EA Frostbite: Henry Allen's GDC 2021 Animation Summit talk, "Environmental and Motion Matched Interactions; 'Madden', 'FIFA' and Beyond!", followed EA's 2017 "N-teract" talk. It covered Frostbite's Interaction Scenario System and "two approaches to integrating motion matching with multi-character interaction, without loss of player control". — [GDC Vault](https://www.gdcvault.com/play/1027465/Animation-Summit-Environmental-and-Motion); [EA Frostbite, "Frostbite presents at GDC and SIGGRAPH 2021"](https://www.ea.com/technology/news/frostbite-presents-at-gdc-and-siggraph-2021); [YouTube recording](https://www.youtube.com/watch?v=TnLhP81r45Q)
- Remedy Northlight: in Alan Wake 2, "all NPCs utilize animation-driven movement combined with new distance-based Motion Matching". This sits alongside a new voxel-based character controller. — [Remedy, "How Northlight makes Alan Wake 2 shine", 2023-11-06](https://www.remedygames.com/article/how-northlight-makes-alan-wake-2-shine). Remedy's Northlight page lists motion matching as an engine feature for using "large amounts of motion capture data". — [Remedy Northlight page](https://www.remedygames.com/northlight)
- IO Interactive Glacier: at Summer Game Fest in June 2025, IOI said 007 First Light adds "a new animation motion-matching system" to Glacier. — [TechRadar (June 2025)](https://www.techradar.com/gaming/007-first-light-io-interactive-hitman-interview-sgf-2025). Wccftech, citing a Digital Foundry interview with IOI, reports motion matching, more full-body interactions, and motion warping for combat and cinematic transitions. — [Wccftech](https://wccftech.com/007-first-light-targets-60-fps-everywhere-except-series-s/). Wikipedia lists the PS5, PC, and Xbox Series release as May 27, 2026. — [Wikipedia: 007 First Light](https://en.wikipedia.org/wiki/007_First_Light)
- O3DE (open-source engine, Linux Foundation; AWS developer Benjamin Jillich): motion matching shipped in EMotion FX as a blend-tree node, with a KD-tree broad phase, a cost-based narrow phase, and debug views. Posted May 12, 2022. — [O3DE blog, 2022-05-12](https://docs.o3de.org/blog/posts/blog-motionmatching/)
- Ubisoft engines (Anvil, Snowdrop): Clavet's GDC 2016 For Honor talk notes that "about 4 or 5 Ubisoft projects" were using the technique in 2016. It also names Michael Büttner (Ubisoft Toronto) and Xavier Lemaire (Ubisoft Montpellier) as working on it. — [Game Anim, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/). I found no public source that names motion matching in Anvil or Snowdrop titles; see Gaps.

### Inferences
- Epic's product direction has been "motion matching inside the selection layer" rather than a standalone node. The evidence: Choosers in 5.4, an experimental Pose Search column in Choosers in 5.7, and a Chooser whose Pose Match Column holds several databases in 5.8. Designers pick a context or database set with rules, and motion matching picks the frame within it. EA's 2023 talk (Context Database, bucketing, priority databases) describes the same split.
- The 5.8 note that Chooser databases default to brute force "since KD Tree usually fails with external filtering" is a concrete production lesson. Spatial acceleration structures conflict with per-query filtering by tag or context, so a design that leans on filtering should keep a brute-force path, ideally SIMD-friendly.
- Multi-character motion matching (Witcher 4 demo in June 2025, then the experimental `MotionMatchMulti` and Interaction Assets in UE 5.8 in June 2026) is the current frontier, and Epic still marks it not for production use as of October 2026. The timing suggests the CDPR partnership drove it, but no source states that link.
- Fortnite's December 2023 rollout shows that switching to motion matching changes game feel, including movement speeds and responsiveness, in ways players notice. Plan design tuning time alongside the animation work.

### Gaps
- I could not open Epic's UE 5.0 to 5.3 release notes, because the release-notes pages returned only a table of contents. The exact version where Pose Search first shipped as Experimental, and whether 5.3 called it "Motion Matching (Experimental)", is unconfirmed.
- The UE 5.4 "Production-Ready" and "all 100 characters plus NPCs" wording comes from search excerpts of Epic's 403-blocked blog and its syndicated copies, not from a direct read.
- No source states whether The Witcher 4 uses Pose Search directly or a CDPR fork, how large its databases are, or whether its multi-character work is what became the UE 5.8 Interaction Assets.
- Ubisoft Anvil and Snowdrop: no talk or official page found confirming motion matching in Assassin's Creed (Origins through Shadows), Ghost Recon Wildlands or Breakpoint, The Division, Avatar: Frontiers of Pandora, or Star Wars Outlaws. A 2019 ResetEra user claimed Ubisoft shipped motion matching before Breakpoint; that is unverified.
- EA "ANT": no source found that describes ANT's role in EA's motion matching.
- Guerrilla Decima (Horizon Forbidden West) and Insomniac (Spider-Man 2): searches found no statement either way.
- The Unity Unite 2025 roadmap (November 2025) content on animation was not verified first-hand, and no 2026 Unity statement on motion matching was found.

## 2. Shipped games that use motion matching: what each team reported

### Takeaway
Talk-level or official confirmation exists for:
- For Honor (Ubisoft Montreal, 2017; GDC 2016 talk)
- The Last of Us Part II (Naughty Dog, 2020; GDC 2021 talk)
- EA titles including Madden and FIFA (GDC 2021 and 2023 talks)
- Fortnite Battle Royale (Epic, Chapter 5, December 2023)
- Alan Wake 2 (Remedy, 2023, NPCs)
- Black Myth: Wukong (Game Science, 2024, UE; talk at an Epic event)
- 007 First Light (IOI, May 2026; interviews)

Teams rarely publish database sizes, clip counts, or team sizes. The published numbers are mostly qualitative ("large anim budget", "10–15 minutes" of prototype data, "500+ animations" in Epic's sample). For the Ubisoft open-world titles, Spider-Man 2, Call of Duty, Hitman (pre-2026), and Control, I found no confirmation.

### Cited Findings

**For Honor (Ubisoft Montreal; shipped February 2017): confirmed by GDC 2016 talk**
- Simon Clavet presented "Motion Matching and The Road to Next-Gen Animation" at GDC 2016. — [GDC Vault](https://gdcvault.com/play/1022985/Motion-Matching-and-The-Road); [Game Anim recap, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/). One write-up calls For Honor (2017) the first game to use the technique. — [jiahaoli.org, 2023-05-02](https://jiahaoli.org/blog/2023/05/02/motion-matching-in-unreal-engine-52-motion-matching-in-for-honor/)
- Clavet was the physics programmer on Assassin's Creed III and began exploring motion matching after that project. Animation director: Khai Nguyen. — [Game Anim, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)
- Reported details:
  - Each hero has its own animation set, with nothing shared between movesets, and a "large anim budget"; no count was given.
  - Matching uses about 10 factors: a few bones, local velocity, feet positions and velocities, and weapon position. Trajectory features are future position, orientation, and velocity.
  - The search is brute force, with blending decisions precomputed offline.
  - Clavet declined to discuss memory. LOD reduces update cost, enough for 100+ characters on screen.
  - Source: [Game Anim, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)
- Earlier, Michael Büttner (Ubisoft Toronto) presented motion matching at nucl.ai 2015. — [Game Developer, "Most inspiring game animation tech talks of 2016"](https://www.gamedeveloper.com/programming/most-inspiring-game-animation-tech-talks-of-2016)

**The Last of Us Part II (Naughty Dog; 2020): confirmed by GDC 2021 talk**
- "Motion Matching in 'The Last of Us Part II'" was given by Michal Mach and Maksym Zhuravlov (Principal Gameplay Animator) on Friday, July 23, 2021, at the virtual GDC 2021. The session description says the team "decided to take a leap of faith and switch to at that time pretty experimental animation system called Motion Matching". It traces "early excitement, through frustration, to eventual worry" because "not everything went according to plan". — [GDC Vault](https://www.gdcvault.com/play/1027118/Motion-Matching-in-The-Last); [Michal Mach on X (June 2021)](https://x.com/gameplayAnim/status/1407520873465008129); [Naughty Dog blog, "Naughty Dog at GDC 2021"](https://www.naughtydog.com/blog/naughty_dog_at_gdc_2021) (slides offered as a 3.8 GB PowerPoint)
- Mach's NPC animation reel says he spearheaded motion matching with John Bellomy. It was used mainly for locomotion, with some use in traversal, melee, and cover. This is from a search excerpt; Vimeo blocked the fetch. — [Vimeo: TLOU Part II NPC Animation / Supervision Reel (2020)](https://vimeo.com/433024043)
- Co-director Anthony Newman described it as "technology for characters moving through a space like running, walking, jumping". — [GameSpot (2019)](https://www.gamespot.com/articles/the-last-of-us-2-has-an-awesome-improvement-you-mi/1100-6470118/) (search excerpt); see also [Push Square, September 2019](https://www.pushsquare.com/news/2019/09/the_last_of_us_2s_motion_matching_system_sounds_like_the_future_of_animation_in_games)

**EA: UFC, Madden, FIFA / EA Sports FC**
- EA Sports UFC 3 (February 2018): Geoff Harrower's GDC 2018 talk, "Real Player Motion Tech in 'EA Sports UFC 3'", describes rebuilding gameplay and animation around "Real Player Motion". It covers path generation that matches captured motion to "motion requests sent to an RPM database query", an "RPM Event System" for temporal constraints on queries, and layering for pose variety. The description does not use the words "motion matching". — [GDC Vault](https://gdcvault.com/play/1025228/Real-Player-Motion-Tech-in). Forbes reported more than 5,000 new animations. — [Forbes, 2017-11-11](https://www.forbes.com/sites/brianmazique/2017/11/11/ea-ufc-3-new-real-player-motion-gameplay-trailer-released/). UFC 4 (2020) kept RPM. — [Wikipedia: EA Sports UFC 4](https://en.wikipedia.org/wiki/EA_Sports_UFC_4)
- Madden and FIFA: Henry Allen's GDC 2021 talk uses Madden and FIFA as case studies for combining motion matching with multi-character interactions (N-teract). — [GDC Vault](https://www.gdcvault.com/play/1027465/Animation-Summit-Environmental-and-Motion)
- Madden NFL 19 (2018): EA said the locomotion system "has been rebuilt using technological innovations from across a variety of EA games" under the "Real Player Motion" name. It does not say "motion matching" explicitly. — [EA, "Madden Deep Dive: Gameplay and Real Player Motion"](https://www.ea.com/news/gameplay-deep-dive?isLocalized=true)
- FIFA 22 (2021), HyperMotion: an "ML-Flow" machine-learning model trained on about 8.7 million frames of 11v11 Xsens match capture, plus more than 4,000 new animations. It was limited to PS5, Xbox Series, and Stadia. EA describes it as a learned model that generates animation in real time, not as motion matching. — [VGC (July 2021)](https://www.videogameschronicle.com/news/fifa-22-release-date-and-machine-learning-animations-announced-in-first-trailer/); [80.lv](https://80.lv/articles/ea-uses-machine-learning-for-animations-in-fifa-22). A mocap vendor blog claims FIFA adopted motion matching for locomotion "around FIFA 22". This is secondary and unverified. — [MocapOnline blog](https://mocaponline.com/blogs/mocap-news/motion-matching-games-guide)
- The title "Motion Matching at EA: 5 Years Later" (GDC 2023) implies EA production use from about 2018. Slide 2 shows game logos as images, which I could not read. — [GDC 2023 slides](https://media.gdcvault.com/gdc2023/Slides/Motion+Matching+at+EA_Delannoy_JC.pdf)

**Fortnite Battle Royale (Epic; Chapter 5, December 2023): official**
- Motion Matching and Procedural Layering on all platforms, used on all 100 characters plus NPCs. The overhaul took about a year, and players pushed back on the movement changes. See Section 1. — [80.lv, 2023-12-04](https://80.lv/articles/enjoy-improved-animations-in-fortnite-chapter-5); [Epic UE 5.4 post](https://www.unrealengine.com/blog/unreal-engine-5-4-is-now-available)

**Alan Wake 2 (Remedy; October 2023): official**
- All NPCs use animation-driven movement with "distance-based Motion Matching", which "gives us more control over how and when animations are used". — [Remedy, 2023-11-06](https://www.remedygames.com/article/how-northlight-makes-alan-wake-2-shine). A Hacker News commenter still saw foot sliding in Remedy's example clips. — [Hacker News thread (Nov 2023)](https://news.ycombinator.com/item?id=38180846)

**Black Myth: Wukong (Game Science; August 2024; Unreal Engine 5): confirmed by talk**
- At an Epic "Unreal Circle" technology salon in Shenzhen on March 27 (year not confirmed; most likely 2025), Game Science client-technology lead Zhao Wenyong said the game's locomotion uses Motion Matching.
  - He chose it to handle accelerations, decelerations, back-and-forth running, and on-the-spot turns.
  - He said the volume of animation required "a simple enough pipeline that can be outsourced".
  - He also discussed non-standard rigs: forms with "double right feet", quadrupeds, insects, and snakes.
  - Sources: search excerpts of [ElectroDealPro](https://electrodealpro.com/game-science-this-technology-allows-black-myth-wukong-to-solve-many-action-problems/), which returned 404 when fetched. Engine: [Wikipedia: Black Myth: Wukong](https://en.wikipedia.org/wiki/Black_Myth:_Wukong)

**007 First Light (IO Interactive; May 27, 2026; Glacier): confirmed by studio interviews**
- New motion matching system in Glacier, plus motion warping and full-body interactions. — [TechRadar (June 2025)](https://www.techradar.com/gaming/007-first-light-io-interactive-hitman-interview-sgf-2025); [Wccftech](https://wccftech.com/007-first-light-targets-60-fps-everywhere-except-series-s/); [Wikipedia](https://en.wikipedia.org/wiki/007_First_Light). A personal portfolio lists "Motion Matching tools in Glacier Editor" among the responsibilities of an IOI animation tech programmer. — [pacojq.xyz](https://www.pacojq.xyz/games/007-first-light)

**Unconfirmed or rumored**
- Ghost Recon Wildlands (March 2017) and Breakpoint (2019): no primary source found. A ResetEra user claimed Ubisoft had already shipped motion matching. — [ResetEra (2019)](https://www.resetera.com/threads/ghost-recon-breakpoint-announce-trailer-gameplay-walkthrough-oct-4th-john-bernthal-raids.116061/page-4)
- Star Wars Outlaws (2024) and Avatar: Frontiers of Pandora (2023), both Snowdrop: no mention of motion matching found. Coverage of Outlaws focuses on Nix's mocap puppet; one animator's reel credits Nix as keyframed. — [Game Developer](https://www.gamedeveloper.com/art/take-a-look-at-nix-s-motion-capture-puppet-from-star-wars-outlaws); [Vimeo reel](https://vimeo.com/1125856013)
- Spider-Man 2 (2023): no Insomniac statement found. Modders describe its traversal as fixed pre-authored sequences. — [Nexus Mods](https://www.nexusmods.com/marvelsspiderman2/mods/1659)
- Call of Duty: no source found. Hitman World of Assassination: no source found; IOI presented motion matching as new in 007 First Light. — [TechRadar](https://www.techradar.com/gaming/007-first-light-io-interactive-hitman-interview-sgf-2025)
- Control (2019): no source found. Remedy calls Alan Wake 2's motion matching "new". — [Remedy](https://www.remedygames.com/article/how-northlight-makes-alan-wake-2-shine)
- Witcher 4: a tech demo only, so not a shipped game. Not expected before 2027. — [Tribune](https://tribune.com.pk/story/2549319/witcher-4-tech-demo-shows-unreal-engine-5s-latest-features)

### Inferences
- The confirmed adopters split into two styles:
  - Player-character locomotion first, as in For Honor, TLOU2, Fortnite, and Black Myth.
  - NPC locomotion first, as in Alan Wake 2, and TLOU2's NPC-heavy use.
- Interactions (EA's multi-character work, Witcher 4, UE 5.8) are where studios are now taking it.
- No team published a database size in minutes or MB. The best public scale markers are Epic's sample (500+ animations, June 2024), Zadziuk's prototype (10–15 minutes of untouched data, 2016), and Motorica's vendor claim of more than 40 minutes for a locomotion set (Section 3).
- Several confirmed adopters built motion matching into proprietary engines years before Unreal's productization: Ubisoft in 2015–2016, EA around 2018, Naughty Dog in 2020, Remedy in 2023, and IOI in 2026. Engine-level motion matching is still unusual: in-house teams keep building their own.

### Gaps
- No public figures were found for TLOU2's database size, clip count, mocap minutes, or team size. The GDC Vault video and slides were unreachable, and the reel description was blocked.
- No database sizes, minutes, or team sizes for For Honor, Fortnite, Black Myth, Alan Wake 2, or 007 First Light.
- It is unconfirmed whether EA's UFC "Real Player Motion" is motion matching in the strict sense, though it is a database-query system. It is also unconfirmed which EA titles appear in the 2023 slide logos.
- The year of the Game Science Shenzhen talk could not be verified.
- No primary confirmation for Assassin's Creed, Ghost Recon, Snowdrop titles, Spider-Man 2, Call of Duty, Control, or Hitman 3.

## 3. Middleware and tools

### Takeaway
Since 2023 the third-party market for motion matching runtimes has mostly collapsed into free, unsupported releases:
- Motion Symphony for Unreal is now free with "SUPPORT IS NO LONGER AVAILABLE".
- MxM for Unity was released free and open source by its author, who left the industry; a community fork maintains it.
- Unity's Kinematica stopped at 0.8 in 2020.

The live commercial activity is on the data side. Motorica sells generative-AI-made motion matching datasets, and Ubisoft La Forge published Learned Motion Matching (2020) to cut runtime memory. Cascadeur offers no motion matching feature.

### Cited Findings
- Motion Symphony (Animation Uprising, Unreal plugin):
  - Version 1.x "will receive limited support and will not be updated for newer versions of UE beyond 5.2".
  - Version 2.0 supports UE 5.3+. It deprecates its own mirroring profiles in favor of Epic's Mirror Data Tables.
  - The docs now state "Motion Symphony is Now FREE. SUPPORT IS NO LONGER AVAILABLE."
  - Sources: [Motion Symphony docs (wikiful), F.A.Q.](https://www.wikiful.com/@AnimationUprising/motion-symphony/f-a-q); [Motion Symphony 2.0 Update](https://www.wikiful.com/@AnimationUprising/motion-symphony/motion-symphony-2-0-update); [Animation Uprising GitHub](https://github.com/Animation-Uprising)
- MxM, "Motion Matching for Unity" (Kenneth Claassen / Vault Break Studios):
  - Released free and open source after the author left the game industry. He said no support would be provided and the package would likely not be updated further.
  - The community fork, Frost-Blade-Studios/Motion-Matching-for-Unity, targets Unity 2022.3 LTS or newer. Its docs say it is MIT-licensed, which I did not verify against the LICENSE file.
  - A community write-up reports it works on Unity 6.
  - Sources: [Frost-Blade-Studios GitHub](https://github.com/Frost-Blade-Studios/Motion-Matching-for-Unity); [MxM Community Fork docs](https://motion-matching-for-unity.readthedocs.io/en/latest/); [Unity Asset Store listing](https://assetstore.unity.com/packages/tools/animation/motion-matching-for-unity-145624); [note.com write-up](https://note.com/creator_gaku/n/n875be540368e?hl=en)
- Kinematica (Unity): suspended in March 2021 and never resumed. See Section 1. — [Unity Discussions](https://discussions.unity.com/t/unite-now-kinematica-democratizing-motion-matching-for-character-animation/788112?page=9)
- Motorica (generative AI animation):
  - Supports generating "motion matching datasets" for Unreal. Its "Motion Factory" produces a motion matching set with "more than 40 minutes" of animation, which is a vendor claim.
  - It released a free motion matching dataset "organized for Unreal Engine Pose Search" (walks, runs, starts, stops, turns).
  - A Quick Import plugin retargets the data and sets up Pose Search databases.
  - Sources: [Motorica docs: Unreal Engine integration](https://docs.motorica.com/documentation/integrations/unreal-engine); [Motorica blog: free motion matching dataset](https://motorica.com/blog/download-the-free-motorica-motion-matching-dataset); [Motorica blog: "Motion Matching Made Simple"](https://motorica.com/blog/unreal-engine-motion-matching-made-simple-episode-1-faster-animation-workflows-with-motorica); [LinkedIn WIP post, UE 5.3 MM with Motorica data (around Dec 2023)](https://www.linkedin.com/posts/alex-moon-bb529335_wip-ue53-motion-matching-using-motorica-activity-7147716103169855488-TZrE)
- Cascadeur (Nekki): I found no motion matching feature. Cascadeur 2025.2 (July 2025) added motion-style presets to AI inbetweening, up to 120 frames. Cascadeur 2026.1 added an Unreal Engine Live Link and an AI Root Motion Tool. — [Digital Production, 2025-07-31](https://digitalproduction.com/2025/07/31/cascadeur-2025-2-adds-motion-styles-tangent-editing-and-automatic-fulcrum-detection/); [Cascadeur blog: 2026.1](https://cascadeur.com/blog/view/cascadeur-2026-1-new-renderer-ue-live-link)
- Ubisoft La Forge, Learned Motion Matching (published July 30, 2020; SIGGRAPH 2020):
  - Motion matching memory "only gets worse the more the system grows", and a AAA production could need "many hundreds of megabytes, perhaps gigabytes".
  - In their expanded test (47 extra joints, about 30 locomotion styles), plain motion matching used about 590 MB. Learned Motion Matching's three networks (Decompressor, Stepper, Projector) took about 17 MB, or 8.5 MB quantized to 16-bit: roughly 70 times less.
  - It is meant to leave the animator workflow unchanged: animators build with motion matching, and LMM is a final compression step with constant CPU cost.
  - The post names no shipped game that uses it.
  - Sources: [Ubisoft La Forge, "Introducing Learned Motion Matching", 2020-07-30](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching); [ACM paper](https://dl.acm.org/doi/10.1145/3386569.3392440)
- Ubisoft La Forge also presented "Ragdoll Motion Matching" at the GDC 2020 Machine Learning Summit, which combines physics with data-driven animation. — [YouTube: ML Summit, Ragdoll Motion Matching](https://www.youtube.com/watch?v=JZKaqQKcAnw)
- Epic's own sample data, the GASP set of 500+ animations, is free on Fab and is widely used as a de facto reference dataset. — [Fab listing](https://www.fab.com/listings/880e319a-a59e-4ed2-b268-b32dac7fa016?lang=en); [GameFromScratch, 2024-06-12](https://gamefromscratch.com/epic-games-release-500-free-aaa-quality-game-animations/)

### Inferences
- Once Epic shipped a free, production-ready Pose Search in 2024, the paid Unreal plugin market was undercut, and the plugins Motion Symphony and MxM went free and unsupported. The lasting commercial value now sits in data (capture, generation, retargeting into the database format) and in memory reduction (LMM), not in the search runtime.
- For a new engine, an importer for common dataset layouts (GASP-style databases, Motorica exports, dance-card captures) may matter as much as the runtime.

### Gaps
- No licensing or usage figures for Motorica in shipped games.
- No evidence found that any shipped game uses Learned Motion Matching. Ubisoft La Forge has not, in what I found, named one.
- I found no other commercial motion matching middleware beyond those above. RADiCAL, DeepMotion, and Inworld are not motion matching; I did not survey them.

## 4. Production costs and workflow: capture, animator workflow, debugging, memory, iteration, and hybrids with state machines

### Takeaway
Every published account says the algorithm is easy and the data is hard:
- Capture must be planned with "dance cards".
- Databases must be split or tagged so the right frames win.
- Teams need dedicated tools to see why a frame was chosen: cost breakdowns, candidate overlays, and replay or rewind.

No production in this survey runs motion matching alone. Every one keeps game logic or state machines above it and layers IK, procedural, or physics passes below it. Naughty Dog's own talk describes moving from excitement to frustration to worry. Unity chose not to ship motion matching in its next animation system. No studio, in what I found, publicly reported abandoning it.

### Cited Findings

**Capture planning and volume**
- For Honor (GDC 2016):
  - Stuntmen follow "dance cards" in seven categories: walks and runs, small repositions, starts and stops, circles, plant-and-turns at 45, 90, 135, and 180 degrees, square strafes with 90-degree plants, and 180-degree strafe plants.
  - Volume is limited mainly by performer stamina, and fatigued late-session takes were recommended.
  - Clavet himself directed exploratory sessions, including about five minutes of random stumbling.
  - Extra transitions can be captured and added late in development.
  - Source: [Game Anim, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)
- Ubisoft Toronto prototype (Zadziuk, GDC 2016 Animation Bootcamp, March 14, 2016):
  - The first test used "around 10/15 minutes of untouched data" in one file.
  - The work brought a "mentality shift, more work done on the mocap stage".
  - Listed failures: editing "can be Labour Intensive", "Large amounts of wasted data" at first, and "Cost and availability of mocap".
  - Listed limits: human-like rigs only, full-body only, quality traded against precise response, and locomotion-only use so far.
  - Planned future work: a hybrid that layers motion matching with a state machine.
  - Michael Büttner created the technique.
  - Sources: [archive.org transcript](https://archive.org/stream/GDC2016Zadziuk/GDC2016-Zadziuk_djvu.txt); [GDC Vault](https://www.gdcvault.com/play/1023478/Animation-Bootcamp-Motion-Matching-The); [Zadziuk, LinkedIn "Dance Card breakdown"](https://www.linkedin.com/pulse/motion-matching-dance-card-breakdown-kristjan-zadziuk)
- O3DE (May 12, 2022) recorded its sample locomotion database with Zadziuk-style dance cards "that cover a full range of motion while limiting redundant data". — [O3DE blog](https://docs.o3de.org/blog/posts/blog-motionmatching/)
- Black Myth: Wukong chose motion matching partly because the animation volume needed a pipeline "that can be outsourced". — [ElectroDealPro (search excerpt)](https://electrodealpro.com/game-science-this-technology-allows-black-myth-wukong-to-solve-many-action-problems/)

**Animator workflow changes**
- For Honor:
  - Animators place events at key moments, such as a sword connecting, instead of chopping clips or building cycles.
  - Clips are grouped with tags (Tired, Heavy Attack) and attributes (stance, range, attack type, outcome).
  - Animators own two-character moves (throws, finishers) that drive displacement directly.
  - Source: [Game Anim, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)
- EA (GDC 2023, Delannoy):
  - Putting every animation in one database causes "so much complexity in a single place" and makes it "difficult to get the desired frame to play since some assets are very similar", for example a start animation versus acceleration out of a turn.
  - The fixes were "very carefully authoring animations so they will exactly match requests in game" and "reducing the amount of selectable frames".
  - Asset tags (starts, stops, cycles, turns, idles) work but are "time consuming and error prone", with animators "constantly opening assets to view tags".
  - "Bucketing", meaning separate databases per category, makes per-database parameters and feature weights easier to vary, and dropping an asset into a database is faster than tagging it.
  - Tags and buckets coexist, so teams can pick ("Give your team options").
  - Source: [GDC 2023 slides](https://media.gdcvault.com/gdc2023/Slides/Motion+Matching+at+EA_Delannoy_JC.pdf)
- EA mechanisms (same slides):
  - Priority Databases: a separate set of preferred assets, loadable on demand, taken if their best cost is below a threshold. They let a team swap in a new forward run "without creating a full locomotion set".
  - Tags: `DoNotChoose`, and `NextNaturalOnly`, which keeps the search from skipping anticipation and follow-through segments.
  - A "Root Motion Weight" blends 0–1 between physics or motion-model displacement ("consistent motion feel, lower fidelity") and animation root motion ("higher fidelity, can feel inconsistent"), and can change at runtime.
- Unreal (UE 5.8):
  - `UAnimNotifyState_PoseSearchSamplingRange` lets animators mark which part of a clip a database includes.
  - Disabled database animations are stripped at cook.
  - The editor caps preview actors at 50 by default.
  - Source: [UE 5.8 release notes](https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5-8-release-notes?lang=en-US)
- Learned Motion Matching's stated goal is to leave the animator's motion matching workflow unchanged, with compression as a final step. — [Ubisoft La Forge, 2020-07-30](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)

**Runtime tuning reported by teams**
- For Honor:
  - Don't timescale animations more than 10% faster or 20% slower; let the character slide and fix it with foot IK.
  - Stop and start cost about 1 m of control lag, and predictable control was preferred over raw responsiveness.
  - Toes lock via sockets, hip IK handles slopes, and spine pitch handles upper-body height.
  - A slider trades fidelity against responsiveness.
  - Source: [Game Anim, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)
- O3DE runs the search "a few times per second rather than every game tick". — [O3DE blog, 2022-05-12](https://docs.o3de.org/blog/posts/blog-motionmatching/). EA notes motion matching "can be run every frame". — [EA GDC 2023 slides](https://media.gdcvault.com/gdc2023/Slides/Motion+Matching+at+EA_Delannoy_JC.pdf)

**Debugging tools**
- Unreal: Rewind Debugger with a Pose Search track, per-candidate cost and channel breakdown, heat map, and viewport overlay of the candidate pose and trajectory. — [Epic docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-debugging-in-unreal-engine)
- EA: "Diagnostics" panels, debug visualization of the candidate pose and trajectory, "Trajectory of all top candidates", and a "Replay System". — [GDC 2023 slides, pp. 70–78](https://media.gdcvault.com/gdc2023/Slides/Motion+Matching+at+EA_Delannoy_JC.pdf)
- O3DE: debug views of joint velocities, trajectories, query values, feature costs over time, and performance. It also offers per-feature histograms that expose dataset imbalance (for example, more forward clips than strafe or backward clips) and 2D PCA scatter plots for coverage. "High feature costs during a visual glitch suggest the database lacks a suitable motion." — [O3DE blog, 2022-05-12](https://docs.o3de.org/blog/posts/blog-motionmatching/)

**Memory budgets**
- O3DE: one hour at 30 Hz is 108,000 frames. A 59-value feature schema gives about 6.4 million floats, about 24.3 MB, for the feature matrix alone. — [O3DE blog, 2022-05-12](https://docs.o3de.org/blog/posts/blog-motionmatching/)
- Ubisoft La Forge: about 590 MB for a large motion matching controller, against 8.5–17 MB with LMM. AAA could need "many hundreds of megabytes, perhaps gigabytes". — [Ubisoft La Forge, 2020-07-30](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)
- Unreal's memory levers: sample rate, channel and sample counts, PCA components, pose pruning similarity threshold, and data padding. — [Epic docs (UE 5.8)](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- For Honor's Clavet declined to discuss memory in 2016. — [Game Anim](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)

**Hybrids with state machines and other systems**
- For Honor: "Game logic still runs as a state machine, while the matching system chooses the animation clips." — [Game Anim, 2016-05-03](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)
- EA:
  - A "Match Node" inside a regular graph queries the database "upon entry" and then plays the asset to completion.
  - A "Match Gate Node" takes a transition only if the best frame's cost is below a threshold.
  - A "Context Database" filters valid animations by game conditions, because plain motion matching "doesn't handle conditions on game context".
  - Motion warping to targets adds a "Distance To Target Cost".
  - Takeaway slide: "Leverage MM outside of locomotion".
  - Source: [GDC 2023 slides](https://media.gdcvault.com/gdc2023/Slides/Motion+Matching+at+EA_Delannoy_JC.pdf)
- Unreal GASP (2024–2026): Choosers and Proxy Tables select among several databases. The 5.7 and 5.8 Mover pawn first used a "State Machine-driven blendstack", which 5.8 reworked into a Chooser with a Pose Match Column. A State Tree drives Smart Object interactions. — [The Rookies (2026)](https://www.therookies.co/blog/headlines/unreal-engines-game-animation-sample-project); [uhiyama-lab](https://uhiyama-lab.com/en/notes/ue/motion-matching-game-animation-sample/)
- Fortnite layers procedural animation (the "Copy Motion" node) and, on high-end platforms, physics-based animation on top of motion matching. — [80.lv, 2023-12-04](https://80.lv/articles/enjoy-improved-animations-in-fortnite-chapter-5); [Epic support (search excerpt)](https://eoshelp.epicgames.com/s/article/Using-Copy-Motion-node-for-layering-with-Motion-Matching?language=en_US)
- Alan Wake 2 combines "animation-driven movement" with "distance-based Motion Matching" for NPCs. — [Remedy, 2023-11-06](https://www.remedygames.com/article/how-northlight-makes-alan-wake-2-shine)

**Iteration time and team effort**
- Fortnite's locomotion overhaul took about a year. — [80.lv, 2023-12-04](https://80.lv/articles/enjoy-improved-animations-in-fortnite-chapter-5)
- Zadziuk (2016) lists "simplified implementation and rapid prototyping" with "no need for state machines" as successes. — [archive.org transcript](https://archive.org/stream/GDC2016Zadziuk/GDC2016-Zadziuk_djvu.txt)
- EA (2023): "Getting Motion Matching into production can be very challenging." — [GDC 2023 slides](https://media.gdcvault.com/gdc2023/Slides/Motion+Matching+at+EA_Delannoy_JC.pdf)
- Naughty Dog (2021): "not everything went according to plan". — [GDC Vault](https://www.gdcvault.com/play/1027118/Motion-Matching-in-The-Last)
- An informal developer forum thread argues that GDC talks "consistently undersell" the work of authoring a well-behaved database. This is an informal source. — [Moonjump Studios forum](https://moonjump.com/forum/animating/motion-matching-feels-like-magic-but-is-it-actually-feasible-outside-of-aaa-2cc459)

**Cases of "not worth it" or deprioritization**
- Unity suspended Kinematica (2021), cited integration problems such as Mecanim incompatibility and no compression (2023), and left motion matching out of its new system's first release (May 2025). — [Unity Discussions](https://discussions.unity.com/t/unite-now-kinematica-democratizing-motion-matching-for-character-animation/788112?page=9); [Unity Discussions, p.3](https://discussions.unity.com/t/animation-status-update-q1-2025-gdc-roadmap/1614718?page=3)

### Inferences
- The production pattern across Ubisoft (2016), EA (2023), Epic (2024–2026), and Remedy (2023) has four layers:
  1. Gameplay logic or a state machine picks the context.
  2. A rule layer (Chooser, Context Database, buckets) picks the eligible databases.
  3. Motion matching picks the frame by cost.
  4. Post-processing (foot locking, IK, warping, procedural layers, physics) repairs the result.
- This fits Byte-Engine's AGENTS.md rule to separate concerns by layer. The search can be a pure function from a query and a filtered candidate set to the best frame. Selection rules, blending, and IK stay separate.
- Data tooling and debugging carry at least as much weight as the search. Every team that talked about it built cost breakdowns, candidate overlays, and replay. Byte-Engine already has an inspector with data streams, which could host a pose-search trace.
- Memory scales linearly with minutes of data and feature width: about 24 MB per hour at 59 floats per frame (O3DE). A locomotion-only set of 10–40 minutes plausibly fits in a few MB of features plus the pose data. Ubisoft's hundreds of MB to GB figures apply to many-style, many-character AAA sets.
- Threshold-gated transitions recur across teams: EA's Match Gate and priority databases, Unreal's continuing-pose bias. A runtime should expose "only switch if better by X" and per-database biases from the start.

### Gaps
- No team published per-character mocap minutes, capture-day counts, or animator headcount. TLOU2's talk likely has these, but the Vault was unreachable.
- No published CPU timings (ms per search) for any shipped game.
- No public post-mortem found of a studio that removed motion matching from a shipped or late-stage project, beyond Unity's deprioritization.
- EA's diagnostics and replay tooling exists only as slide images. The PDF text layer shows only headings, so their content is unknown.
