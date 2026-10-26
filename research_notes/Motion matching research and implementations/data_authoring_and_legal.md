# Motion matching: data, capture and authoring, patents, and licenses (state as of October 2026)

Scope note: these notes cover data sources, capture and authoring practice, patents, and license terms. They do not survey open-source implementations in depth (another researcher covers those). **Nothing here is legal advice.** It is factual research by a non-lawyer. Patent scope, validity, and infringement can only be judged by a qualified patent attorney. License readings below are summaries of the quoted text and may be wrong at the edges. Every expiry date is either Google Patents' own estimate (which Google labels "an assumption and not a legal conclusion") or my own arithmetic from the filing date plus the patent term adjustment printed on the patent. Both ignore maintenance-fee lapses and anything else that changed after the snapshot.

Method note: I pulled Google Patents bibliographic data and claims through its public endpoints until it began returning HTTP 503 (rate limiting) partway through. I read the remaining claims by optical character recognition (OCR) of the scanned USPTO PDFs (`image-ppubs.uspto.gov`), so small OCR errors are possible in quoted claim text. I read the Unreal EULA, the Epic Content License Agreement, the Fab license summary, and the Fab listing for the Game Animation Sample from Wayback Machine snapshots dated 5–12 October 2026, because the live Epic and Fab pages blocked automated fetches.

---

## 1. Which datasets suit locomotion motion matching, and what do their licenses allow?

### Takeaway
Most of the high-quality locomotion sets have noncommercial licenses (LaFAN1, Bandai Namco, ZeroEGGS, AMASS, Motorica) or are tied to Unreal (the Game Animation Sample). These sets cannot go into an MIT-licensed repository, and they cannot ship in a commercial product. The only sources I found that are both redistributable and usable commercially are the CMU database ("free for all uses", may be "copied, modified, or redistributed", must not be resold directly), 100STYLE (CC BY 4.0, attribution required), and CC0 hand-made libraries such as Quaternius' Universal Animation Library. LaFAN1 remains the best quality for local, noncommercial development (it is what Daniel Holden's reference demo uses). Mixamo allows commercial games but forbids redistributing raw files and training ML models on them.

### Cited Findings

**Ubisoft La Forge Animation Dataset (LaFAN1)**
- Content: 5 subjects, 77 sequences, 496,672 frames at 30 fps (~4.6 hours, ≈276 min). Themes include walk (12 sequences), run (4), sprint (2), obstacles (17), aiming (5), dance (8), fall and get up (6), fight, jumps, and push/stumble. Animation data is BVH, named `[theme][take number]_[subject ID].bvh`. The repository also has `c3d` and `fbx` folders and needs Git LFS — [LaFAN1 GitHub README](https://github.com/ubisoft/ubisoft-laforge-animation-dataset)
- Exact license: "Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International Public License" (the full text is in `license.txt`). The README says "This dataset can be used under the Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International Public License (see license.txt)." Notice: "© [2018] Ubisoft Entertainment. All Rights Reserved." — [LaFAN1 GitHub](https://github.com/ubisoft/ubisoft-laforge-animation-dataset)
- Under CC BY-NC-ND 4.0 you may "reproduce and Share the Licensed Material, in whole or in part, for NonCommercial purposes only", and you may produce Adapted Material but "You do not have permission under this Public License to Share Adapted Material." NonCommercial means "not primarily intended for or directed towards commercial advantage or monetary compensation" — [license text as published in the ZeroEGGS License.md, same CC BY-NC-ND 4.0 text](https://github.com/ubisoft/ubisoft-laforge-ZeroEGGS/blob/main/License.md)
- Daniel Holden's MIT-licensed Motion-Matching demo builds its database from LaFAN1. Its README says the data "is licensed under Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International Public License (unlike the code, which is licensed under MIT)". The repository nonetheless commits a processed `resources/database.bin` (~64 MB), plus features and network weights derived from that data — [orangeduck/Motion-Matching](https://github.com/orangeduck/Motion-Matching)

**CMU Graphics Lab Motion Capture Database**
- License (no named license; terms quoted from the site): "This dataset of motions is free for all uses." "This data is free for use in research projects. You may include this data in commercially-sold products, but you may not resell this data directly, even in converted form." The site asks for a citation and the acknowledgment "The data used in this project was obtained from mocap.cs.cmu.edu. The database was created with funding from NSF EIA-0196217." — [CMU mocap home page](http://mocap.cs.cmu.edu/)
- FAQ: "The motion capture data may be copied, modified, or redistributed without permission." — [CMU mocap FAQ](http://mocap.cs.cmu.edu/faqs.php)
- Content: "2605 trials in 6 categories and 23 subcategories" (human interaction, interaction with environment, locomotion, physical activities and sports, situations and scenarios, test motions). The same person may appear under several subject numbers, and each subject has its own calibrated skeleton. Toe and hand joints are noisy. Finger and thumb joints are not captured. Lower-numbered subjects are early sessions of lower quality — [CMU mocap home page](http://mocap.cs.cmu.edu/)
- Capture: Vicon, 12 MX-40 cameras at 120 Hz, a volume of about 3 m × 8 m, 41 markers. Formats: ASF/AMC (joint angles; lengths in inches scaled by 0.45, so multiply by 0.056444 to get meters) and C3D. Third-party BVH conversions exist (Bruce Hahn's 3ds Max and MotionBuilder BVHs, hosted at cgspeed) — [CMU mocap FAQ](http://mocap.cs.cmu.edu/faqs.php); [CMU home](http://mocap.cs.cmu.edu/)

**100STYLE (Ian Mason, Edinburgh)**
- Exact license: "Creative Commons Attribution 4.0 International (CC BY 4.0)". For creative or commercial work the page asks for credit such as "The 100STYLE Dataset - Ian Mason". It says users may extract and process the data as they wish — [100STYLE page](https://www.ianxmason.com/100style/)
- Content: over 4 million frames, 100 locomotion styles, one actor. Movement types: forwards walk and run (FW, FR), backwards walk and run (BW, BR), sidestep walk and run (SW, SR), idle (ID), and transitions (TR1–TR3). Unedited BVH at 60 fps, captured with an Xsens system, on a 28-bone skeleton rooted at Hips. Files are named `<StyleName>_<MovementType>.bvh` — [100STYLE page](https://www.ianxmason.com/100style/). (≈18.5 hours if every frame is at 60 fps; my arithmetic.)

**Bandai Namco Research Motion Dataset**
- Exact license: "CC BY-NC 4.0" for both datasets. The Blender visualization scripts are MIT. Notice: "© [2022] Bandai Namco Research Inc. All Rights Reserved" — [Bandai Namco GitHub README](https://github.com/BandaiNamcoResearchInc/Bandai-Namco-Research-Motiondataset)
- Dataset-1: 17 content types (daily activities, fighting, dancing), 15 styles, 36,673 frames. Dataset-2: 10 content types "mainly focusing on locomotion and hand actions", 7 styles, 2,902 clips, 384,931 frames at 30 fps (≈214 min; my arithmetic). The contents are walk, walk-turn-left, walk-turn-right, run, and wave and raise-hand variants. The styles are active, elderly, exhausted, feminine, masculine, normal, and youthful. Data is BVH with JSON annotations. Three professional actors were captured, and the data was post-processed (noise removal, proportion alignment, clipping) — [Bandai Namco README](https://github.com/BandaiNamcoResearchInc/Bandai-Namco-Research-Motiondataset); [Dataset-2 README](https://github.com/BandaiNamcoResearchInc/Bandai-Namco-Research-Motiondataset/tree/master/dataset/Bandai-Namco-Research-Motiondataset-2)

**AMASS (Max Planck)**
- Content: more than 40 hours, over 300 subjects, more than 11,000 motions, unifying 15 optical marker-based datasets (CMU among them) as SMPL body-model parameters — [AMASS home](https://amass.is.tue.mpg.de/). A 2023 review reports a later total of 2,992.34 minutes and 484 subjects — [review, arXiv 2312.06184](https://arxiv.org/pdf/2312.06184). AMASS later added 8.07 minutes of two-person CMU interaction — [AMASS news](https://amass.is.tue.mpg.de/news.html)
- Exact license name: "Dataset Copyright License for non-commercial scientific research purposes". "Any other use, in particular any use for commercial purposes, is prohibited". The license forbids copying, sharing, distributing, re-selling, or sub-licensing (one archive copy is allowed) and bars using the data to train methods for commercial use. Commercial licensing is through ps-license@tue.mpg.de — [AMASS license](https://amass.is.tue.mpg.de/license.html)

**Motorica Dance Dataset (KTH / Motorica)**
- Content: 6 hours of mocap plus audio in 8 dance styles (Hiphop 84 min, Krumping 18, Popping 42, Locking 18, Jazz 52, Charleston 50, Tapping 11, Casual 85). OptiTrack system with 17 Prime41 cameras at 120 fps. BVH retargeted to a single skeleton — [MotoricaDanceDataset README](https://github.com/simonalexanderson/MotoricaDanceDataset)
- Terms (custom; GitHub shows license "Other"): "Use for commercial purposes is not permitted without prior written consent. This includes, without limitation, incorporation in a commercial product, use in a commercial service, or training machine-learning algorithms for commercial purposes." "Unauthorised redistribution of any content from the database is prohibited without written approval". Music has separate copyright holders — [MotoricaDanceDataset README](https://github.com/simonalexanderson/MotoricaDanceDataset). Not locomotion data.

**Ubisoft ZeroEGGS**
- Content: 67 monologue sequences by one female actor, 19 styles, 134.65 minutes total, BVH plus WAV, in original and cleaned folders — [ZeroEGGS GitHub](https://github.com/ubisoft/ubisoft-laforge-ZeroEGGS)
- License file: "Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International Public License" — [ZeroEGGS License.md](https://github.com/ubisoft/ubisoft-laforge-ZeroEGGS/blob/main/License.md). This is gesture data, not locomotion.

**Unreal Game Animation Sample Project (GASP)**
- Content: launched June 12, 2024 with "over 500 AAA-quality animations", Motion Matching locomotion (walk, run, jump, fall), traversal, multiple Motion Matching databases, Choosers and Proxy Tables, and Pose Warping "to address gaps in animation coverage". The blog says "You're free to use all these animations with or without their accompanying animation Blueprints and retarget them to your own characters." — [Epic blog, June 12, 2024](https://www.unrealengine.com/en-US/blog/game-animation-sample)
- Updates: UE 5.7 (Dec 3, 2025) added a Mover-based character, "a new locomotion dataset", and 400 new animations — [Epic tech blog, UE 5.7 update](https://www.unrealengine.com/en-US/tech-blog/explore-the-updates-to-the-game-animation-sample-project-in-ue-5-7) (summary via search; the page itself returned 403). UE 5.8 (Aug 12, 2026) added multi-character motion matching, Pose Search Interaction Assets, and Physics Control — [Epic tech blog, UE 5.8 update](https://www.unrealengine.com/tech-blog/download-the-latest-game-animation-sample-project-now-updated-for-ue-5-8) (summary via search)
- Fab listing (snapshot 2026-10-08): first published 2024-06-11. License offers are "Personal" and "Professional" in the "standard" group at $0, `isAiForbidden: true`. The description says "500+ of game-ready animations compatible with the UE5 Mannequin Skeleton through runtime retargeting" — [Fab listing, Game Animation Sample](https://www.fab.com/listings/880e319a-a59e-4ed2-b268-b32dac7fa016)
- Conflicting claims about engine restriction: CG Channel (June 2024) says the project and its assets are "only licensed for use with Unreal Engine" but "can be used in commercial projects" — [CG Channel](https://www.cgchannel.com/2024/06/get-500-free-game-ready-animations-for-use-in-unreal-engine). The current Fab Standard License summary says "Use the assets with any compatible tools (usage is not limited to Unreal Engine)" but "You may not: Resell or redistribute the asset for free on a standalone basis" — [Fab Standard License summary](https://www.fab.com/eula). I could not find a "UE-Only" marker in the archived Fab listing data. The UE EULA warns that "certain assets that we make available under separate agreements are available for use only with Unreal Engine." — [Unreal EULA §4(a)(i)](https://www.unrealengine.com/en-US/eula/unreal)
- The Epic Content License Agreement (snapshot 2026-10-12) defines "UE-Only Content" as "Licensed Content that is designated as only permitted for use in conjunction with Unreal Engine and Unreal Engine-based products as designated by Epic, such as Twinmotion". It forbids distributing Licensed Content "in source format to third parties except to employees, affiliates, and contractors", forbids standalone sale, and forbids using "NoAI Content" in datasets for, development of, or inputs to Generative AI Programs — [Epic Content License Agreement](https://www.unrealengine.com/en-US/eula/content)

**Mixamo (Adobe)**
- FAQ (last updated Sep 14, 2021): "You can use both characters and animations royalty free for personal, commercial, and non-profit projects including: … Create video games." Mixamo is free with an Adobe ID. It is not available for Enterprise and Federated IDs or in China. Its libraries are "for bipedal humanoids only" — [Adobe Mixamo FAQ](https://helpx.adobe.com/creative-cloud/faq/mixamo-faq.html)
- An Adobe community licensing FAQ (posted by TylerG 3D, Sep 29, 2022) says "the only thing you can't do is distribute the raw character and animation files". It rules out asset-store packages that distribute raw files, and states "The only research application Mixamo content can't be used in is training machine-learning models." It is a community post, not the binding Adobe Terms — [Adobe Community Mixamo licensing FAQ](https://community.adobe.com/t5/mixamo-discussions/mixamo-faq-licensing-royalties-ownership-eula-and-tos/m-p/13234775)

**Rokoko Motion Library free packs**
- 150 free moves were "licensed for use in commercial projects". Paid moves cost $3 or $6 (2020 report) — [CG Channel, Mar 2020](https://www.cgchannel.com/2020/03/get-150-free-mocap-moves-from-rokokos-motion-library). Rokoko support says many assets are free and others cost $3–$20 — [Rokoko support](https://support.rokoko.com/hc/en-us/articles/4410021327121-Getting-Started-Rokoko-Studio-Motion-Library)

**Openly licensed hand-made libraries (redistributable)**
- Quaternius Universal Animation Library: "Free to use in personal, educational and commercial projects. (CC0 License)", 120+ animations, "Locomotion in 8 directions, jog, sprint, push, crawling, swimming, sitting, death animations and more". It uses "a universal humanoid rig" and ships as FBX, glTF, GLB, OBJ, and .blend — [Quaternius UAL](https://quaternius.com/packs/universalanimationlibrary.html). UAL 2 is a separate CC0 kit of 130+ animations — [OpenGameArt UAL 2](https://opengameart.org/content/universal-animation-library-2)
- Mesh2Motion is described as a free, open-source, CC0-licensed rig-and-animate app that includes Quaternius animations — [GameFromScratch](https://gamefromscratch.com/mesh2motion-free-open-source-animation-app/) (secondary; not verified against the repository)

**Summary table (my synthesis of the findings above)**

| Dataset | Locomotion content | Size | Skeleton / format | Exact license | Redistribute in an open-source repo? | Commercial product? |
|---|---|---|---|---|---|---|
| LaFAN1 | walk, run, sprint, obstacles, more | 4.6 h, 30 fps | one BVH skeleton (+fbx, c3d) | CC BY-NC-ND 4.0 | Unmodified only, noncommercial only; no adapted data | No |
| CMU | locomotion category among 2605 trials | 2605 trials, 120 Hz | ASF/AMC, C3D; per-subject skeletons | none named; "free for all uses" | Yes ("may be copied, modified, or redistributed") | Yes, but no direct resale "even in converted form" |
| 100STYLE | FW/FR/BW/BR/SW/SR/ID/TR × 100 styles | >4M frames, 60 fps | 28-bone BVH, Xsens | CC BY 4.0 | Yes, with attribution | Yes, with attribution |
| Bandai Namco 1/2 | DS-2: walk, turns, run × 7 styles | 36,673 + 384,931 frames | BVH + JSON | CC BY-NC 4.0 | Noncommercial only | No |
| AMASS | many (incl. CMU) | >40 h | SMPL parameters | "Dataset Copyright License for non-commercial scientific research purposes" | No | No |
| Motorica Dance | dance only | 6 h, 120 fps | BVH | custom terms | No | No (without consent) |
| ZeroEGGS | gesture only | 134.65 min | BVH + WAV | CC BY-NC-ND 4.0 | Noncommercial, unmodified | No |
| GASP | walk/run/jump/fall/traversal | 500+ (+400 in 5.7) anims | UE5 Mannequin, Unreal assets | Fab Standard License (Personal/Professional); reported UE-only | No (standalone redistribution banned) | Disputed: UE-only per 2024 reports |
| Mixamo | short clips | n/a | Mixamo rig, FBX | Adobe terms | No raw files | Yes, in a product |
| Quaternius UAL | 8-direction locomotion, jog, sprint | 120+ / 130+ anims | universal humanoid rig | CC0 | Yes | Yes |

### Inferences
- For checked-in test fixtures and samples in Byte-Engine's MIT repository, the defensible choices are CC0 (Quaternius), CC BY 4.0 (100STYLE, with an attribution file), and CMU (redistribution allowed; add the acknowledgment; never sell the data "even in converted form"). Everything else should be download-on-demand for local, noncommercial use only, with its license shown to the user.
- LaFAN1's NoDerivatives term likely makes committing a processed database (resampled, mirrored, retargeted, or feature-extracted) derived from LaFAN1 unsafe, even though Holden's MIT repository does so. Byte-Engine should generate such files locally from the user's own download rather than copy that pattern. This is a reading of the CC text, not a ruling.
- 100STYLE is the strongest permissive locomotion set, but it was captured with Xsens IMUs. A community authoring guide warns that IMU suits suit motion matching poorly because of global root accuracy (see Section 2), and the set is style-oriented with one actor. Expect root drift cleanup, and verify it covers starts, stops, and pivots before relying on it.
- CMU is old and noisy, and every subject has a different calibrated skeleton. Use it for format and importer tests (ASF/AMC, BVH conversions) more than as a showcase locomotion set.
- GASP data should be treated as off-limits for Byte-Engine. It cannot be redistributed in any case, it carries a NoAI flag, and 2024 reporting says it is licensed for use only with Unreal Engine.

### Gaps
- I could not open Adobe's binding General Terms or any Mixamo-specific product terms (helpx returned 403 live; only the FAQ was archived). The ban on raw-file redistribution and ML training comes from a 2022 community post.
- I found no current Rokoko Motion Library license text. Whether free Rokoko clips may be redistributed in source form is unknown.
- I could not confirm whether the GASP Fab listing currently carries a "UE-Only" label in Fab's UI. The listing data shows only "standard" Personal and Professional offers. Epic support or the in-launcher license label would settle this.
- CMU total hours: no primary figure found (AMASS's per-dataset table was not retrievable; its PDF returned 403).
- Bandai Namco Dataset-1 frame rate not confirmed (Dataset-2 is 30 fps).
- 100STYLE's coverage of starts, stops, and pivots (inside the "TR" transition files) is not documented on the page.

---

## 2. Capture design: dance cards versus unstructured capture, how much data, coverage, mirroring, retargeting, cleanup

### Takeaway
Classic motion matching started with unstructured "run around" capture. Ubisoft first imported 5–15 minutes of untouched mocap, then structured capture into "dance cards": scripted floor paths repeated per gait (walk, jog, run), split into phases for starts and stops, turns, accelerations, decelerations, speed transitions, and snaking. Each phase is kept to a few minutes because single runs of up to 4 minutes exhaust actors. A minimal working locomotion set is on the order of 8–15 unique minutes. Holden's demo uses about 8.3 minutes of LaFAN1, mirrored and sped up. Shipping sets grow to hundreds of clips and hundreds of MB. Mirroring is widely used to double coverage, but Zadziuk reported in 2016 that it "did not work" for them. Whether it works depends on how symmetric the character and data are.

### Cited Findings
- Clavet's GDC 2016 session (Ubisoft Montreal, For Honor): "They just capture 5 or 10 minutes of a person running around and import it directly into the engine", with no manual organization of starts, stops, and turns. At runtime "they continuously find the frame in the mocap database that simultaneously matches the current pose and the desired future plan, and transition with a small blend time" — [GDC Vault session summary, via search](https://gdcvault.com/play/1022985/Motion-Matching-and-The-Road)
- Zadziuk (Animation Director, Ubisoft Toronto) at GDC 2016, as reported by DualShockers (Mar 15, 2016):
  - The first test was 10–15 minutes of untouched mocap in one file.
  - "Dance Cards" listed every action needed and were repeated for walk, jog, and run, aiming at the fewest moves for the widest coverage.
  - A single run could last up to four minutes, which tired the actor, so the cards were split into phases: starts and stops, turns, accelerations, decelerations, transitions between speeds, and snaking ("especially important").
  - Mirroring "did not work". The system gets to "about 70%" quickly but "the final 30% is difficult".
  - It is data-heavy, and the right data beats all of the data.
  - A locomotion system was built in four hours, versus "at least three months" the traditional way.
  - The matching window is about one second of motion.
  - The "Ape" locomotion was keyframed by hand.
  - Source: [DualShockers](https://www.dualshockers.com/ubisoft-introduces-motion-matching-the-first-step-to-next-gen-animation/)
- Zadziuk later released a video on "the various mocap actions that must be created via the 'dance cards'" and a video of capture for reaction and stumble actions — [GameAnim, Sep 4, 2016](https://www.gameanim.com/?p=14043)
- O3DE (Amazon) credits Zadziuk's team with dance cards, which "describe paths actors move along in a motion capture studio" to cover the full range of motion while minimizing redundant data. It recommends slow and fast locomotion, narrow and wide turns, and acceleration and deceleration. Data volume: 1 hour at 30 Hz is 108,000 frames, ~6.4 million feature values, ~24.3 MB with the default 59-feature schema. The example trajectory is 4 past samples over 0.7 s and 6 future samples over 1.2 s. It suggests histograms and PCA scatter plots to find over-represented or sparse regions, and retaking or adding clips when a glitch coincides with high feature cost — [O3DE motion matching blog](https://docs.o3de.org/blog/posts/blog-motionmatching/)
- A community motion matching authoring guide (MxM for Unity) lists dance card types: rectangle, dial, strafing dial, NSEW dial, straight, circles/spirals, and snakes. It notes IMU suits are generally not recommended because motion matching relies on accurate global root position — [MxM "Guide for Animation Authoring" PDF](https://motion-matching-for-unity.readthedocs.io/en/latest/Motion%20Matching%20-%20Guide%20for%20Animation%20Authoring.pdf) (content seen only through a search summary; I could not extract the PDF text myself)
- Holden's reference demo uses one long walking take and one long running take "somewhat in the style of dance cards". It mirrors them "to increase the coverage" and speeds the data up "by about 10%". Its simulation-bone extraction "works best for long takes which start and stop with a standing pose" — [Holden, "Code vs Data Driven Displacement"](https://theorangeduck.com/page/code-vs-data-driven-displacement)
- The demo's database script uses only three LaFAN1 clips: `pushAndStumble1_subject5.bvh` frames 194–351 (standing idle), `run1_subject5.bvh` frames 90–7086, and `walk1_subject5.bvh` frames 80–7791. It processes each clip mirrored and unmirrored and resamples it with cubic interpolation to 60 Hz while speeding it up by 10% — [generate_database.py in orangeduck/Motion-Matching](https://github.com/orangeduck/Motion-Matching). (That is 14,864 source frames ≈ 8.3 minutes at 30 fps, ≈16.5 minutes with mirroring; my arithmetic.)
- Scale in production: Ubisoft's expanded Learned Motion Matching (LMM) experiment (hands and fingers, ~30 switchable locomotion styles) grew motion matching memory to 590 MB, versus ~17 MB of network weights (8.5 MB quantized) for LMM. Matching features were foot positions and velocities, hip velocity, and a few future trajectory positions and directions — [Ubisoft La Forge, Introducing Learned Motion Matching](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)
- Epic's GASP contrasts "a dense dataset with hundreds of animations versus a sparser set with less than a hundred". It splits animations into multiple databases and uses Pose Warping to fill coverage gaps — [Epic blog](https://www.unrealengine.com/en-US/blog/game-animation-sample)
- Naughty Dog presented "Motion Matching in The Last of Us Part II" at GDC 2021 (Michal Mach, Maksym Zhuravlov) — [Naughty Dog at GDC 2021](https://www.naughtydog.com/blog/naughty_dog_at_gdc_2021). Press reports describe extensive mocap including dogs and horses (no hours given) — [search summary of press coverage, e.g. GamingBolt](https://gamingbolt.com/the-last-of-us-part-2s-new-motion-mapping-technique-provides-incredible-next-gen-leap-in-animations)
- Mirroring in Unreal is driven by a Mirror Data Table. "Setting up and assigning a mirror data table will allow all your assets in your database to access the mirrored version of the data. This is required for mirroring to work with Motion Matching." — [UE Python API, PoseSearchRoledSkeleton](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/PoseSearchRoledSkeleton). UE 5.2 also had a `mirror_mismatch_cost_bias` on the schema — [UE 5.2 PoseSearchSchema](https://docs.unrealengine.com/5.2/en-US/PythonAPI/class/PoseSearchSchema.html). A community answer notes the mirror table must also cover animation curves for GASP — [UE forum](https://forums.unrealengine.com/t/is-it-possible-to-mirror-gasp/2759484)
- Retargeting: GASP animations are "compatible with the UE5 Mannequin Skeleton through runtime retargeting" — [Fab listing](https://www.fab.com/listings/880e319a-a59e-4ed2-b268-b32dac7fa016). Motorica Dance is distributed "retargeted to a single skeleton" — [Motorica README](https://github.com/simonalexanderson/MotoricaDanceDataset). Holden published `motorica-retarget`, a retarget of that dataset onto a common skeleton — [GitHub orangeduck/motorica-retarget](https://github.com/orangeduck/motorica-retarget)
- Cleanup: Bandai Namco applied "noise removal, proportion alignment, and clipping" — [Bandai Namco README](https://github.com/BandaiNamcoResearchInc/Bandai-Namco-Research-Motiondataset). CMU warns that toe and hand joints "tend to be noisy, and may require some smoothing" — [CMU home](http://mocap.cs.cmu.edu/). 100STYLE ships "unedited" BVH — [100STYLE](https://www.ianxmason.com/100style/)
- Clavet's later remark (nucl.ai 2016, as reported): "Let's just directly markup the mocap with the controller inputs that should trigger the moves." — [Game Developer, Jan 24, 2017](https://www.gamedeveloper.com/programming/most-inspiring-game-animation-tech-talks-of-2016)

### Inferences
- A practical Byte-Engine capture plan:
  - Per gait (walk, jog, run, optionally sprint and crouch), record short dance-card phases rather than one marathon take: starts and stops at several angles, 90° and 180° plants and pivots, circles and spirals, snakes, accelerations and decelerations between gaits, and strafing dials for an 8-direction set.
  - Begin and end each take in a standing pose, so root and simulation-bone extraction behave.
  - Budget roughly 8–15 unique minutes for a first playable set, which is what both Ubisoft's first test and Holden's demo used.
- Treat mirroring as an option per database or per clip, plus a mismatch cost bias, not as a global default. This reconciles Zadziuk's "did not work" with the mirroring used by Holden and Unreal. Asymmetric props, handedness, and asymmetric gaits break naive mirroring.
- Optical capture is preferable for motion matching databases (root accuracy). IMU data such as 100STYLE needs root cleanup.

### Gaps
- I could not access the GDC Vault slides or videos (the Vault was redirecting to a maintenance page). Zadziuk's exact dance card diagrams, per-card durations, and planted-turn angles are known only through secondary write-ups.
- I found no public figure for the minutes or hours in For Honor, The Last of Us Part II, or the GASP locomotion databases.
- No primary source located on strafe coverage requirements beyond the MxM card names.

---

## 3. Authoring tools and workflows: tagging, foot contacts, retargeting, augmentation, Unreal Pose Search

### Takeaway
Authoring centers on four things:
- a schema that names and weights the matched features;
- databases of clips, with per-clip trimming and cost biases;
- notify-state tags that exclude, block, or bias frame ranges;
- debugging views that show why a frame was picked.

Unreal exposes all of these to animators. Foot contacts are usually computed automatically from foot or toe velocity thresholds plus filtering. Augmentation means mirroring and time-scaling, with retargeting done offline or at runtime.

### Cited Findings
- **Unreal Pose Search Schema:**
  - Channels include Pose, Trajectory, Velocity, Phase, Heading, Position, Sampling Time, and Group. Each has a Weight, and "Regardless of what values you give to these various weights, the system will automatically normalize them."
  - The schema also sets Sample Rate, Number of Permutations, and Data Preprocessor.
  - Source: [UE docs, Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- **Unreal Pose Search Database:**
  - It accepts Animation Sequences, Animation Composites, and Blendspaces, but not Montages.
  - Per-asset "Exclude from Database Parameters" trim frames at the start and end.
  - The "Continuing Pose Cost Bias" and "Looping Cost Bias" controls shape selection.
  - "Pose Search Mode" is Brute Force, PCAKDTree, or VPTree (experimental).
  - Database Tags act as metadata. Normalization Sets group databases (for example walk and run) so their costs are comparable.
  - Debugging uses the Rewind Debugger and per-channel Debug Color.
  - Source: [UE docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- **Unreal tag and notify states:**
  - "Pose Search: Block Transition" (display name "Pose Search: Block Transition In"): "A pose search search will not return results that overlap this notify, but the animation segment can still play if a previous search result advances into it" — [UE Python API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AnimNotifyState_PoseSearchBlockTransition)
  - "Pose Search: Exclude From Database": "Use this notify state to remove animation segments from the database completely, they will never play or return from a search result" — [UE Python API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AnimNotifyState_PoseSearchExcludeFromDatabase)
  - "Pose Search: Sampling Range" is marked Experimental — [UE Python API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AnimNotifyState_PoseSearchSamplingRange)
  - Others listed are "Override Base Cost Bias", "Motion Matched Branch In", "Sampling Attribute", and "Sampling Effect" — [UE docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine)
- Unreal 5.7 added an experimental Pose Search Column in Choosers. UE 5.8's GASP showcases multi-character motion matching and Pose Search Interaction Assets — [UE 5.7 GASP blog](https://www.unrealengine.com/en-US/tech-blog/explore-the-updates-to-the-game-animation-sample-project-in-ue-5-7); [UE 5.8 GASP blog](https://www.unrealengine.com/tech-blog/download-the-latest-game-animation-sample-project-now-updated-for-ue-5-8) (both via search summaries)
- Pose Search lives under `Plugins/PoseSearch` in Epic's engine API reference (for example `FMirrorDataCache`) — [UE API, Plugins/PoseSearch](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch)
- **Automatic foot contacts (Holden demo):** a contact is set when toe velocity is below a threshold (`contact_velocity_threshold = 0.15`, in m/s after scaling BVH by 0.01). Then a median filter of size 6 is applied as a "majority vote" that "removes small regions where contact is either active or inactive" — [generate_database.py](https://github.com/orangeduck/Motion-Matching)
- **Augmentation (Holden demo):** a mirroring function remaps left and right joints and flips positions and rotations; time-scaling resamples to 60 Hz while playing 10% faster — [generate_database.py](https://github.com/orangeduck/Motion-Matching). The demo uses no tags to separate walking and running — [Motion-Matching README](https://github.com/orangeduck/Motion-Matching)
- Kinematica (Unity, 2018, by Michael Büttner) was pitched as using raw clips or mocap directly "without organizing them in an animation graph" — [Unity blog, Kinematica announcement](https://blog.unity.com/es/technology/announcing-kinematica-animation-meets-machine-learning) (via search summary)
- O3DE's debugging aids are feature histograms and PCA scatter plots of the database — [O3DE blog](https://docs.o3de.org/blog/posts/blog-motionmatching/)

### Inferences
- For Byte-Engine's data model, the Unreal surface suggests the minimum set animators need:
  - per-database schema weights (auto-normalized);
  - per-clip trim ranges and constant cost biases (continuing pose, looping);
  - ranged tags with three effects: exclude, block-entry, and cost bias;
  - per-database mirroring with a mismatch bias;
  - a recorded debugger showing candidate costs per channel.
- Foot contact extraction should be an offline bake step with a threshold and a majority filter, stored as a per-frame bit. It should not be a runtime heuristic.

### Gaps
- I did not find a public source describing how Epic tagged or cleaned the GASP data, or its database split rationale beyond the blog sentence.
- No primary source located for foot-contact heuristics in shipped engines (Unreal's Animation Modifiers and sync-marker tooling were not researched).

---

## 4. Patents on motion matching and closely related techniques

### Takeaway
**This is factual research, not legal advice. Every item needs review by a patent attorney before it is relied on.**

I found no granted patent that claims plain classic motion matching, defined as feature-vector nearest-neighbour search over a mocap database every N frames, using current pose plus future trajectory, followed by a short blend. Classic motion matching was publicly disclosed by Büttner at nucl.ai (20–22 July 2015, Vienna) and by Clavet and Zadziuk at GDC (March 2016). It descends from Motion Fields (Lee et al., 2010) and Motion Graphs (Kovar et al., 2002). These disclosures predate every relevant patent below except two EA filings with a 27 March 2015 provisional.

The patents closest to Byte-Engine's work are these:
- **Unity's US 10,888,785 B2 and US 10,717,004 B2** (Büttner; priority June 2018; expiry ~2039). The continuation's claim 1 is broad: a weighted set of active "atoms", a dominant atom, a motion fragment whose future trajectory is replaced by the predicted trajectory, a search for the most similar fragment, and posing by a weighted combination of active atoms' poses. A European counterpart, EP3807846B1, is listed.
- **EA's local-bone-phase motion matching patents** (US 11,562,523 B1 and US 11,995,754 B2, expiry ~2041) and **EA's periodic-autoencoder motion matching patent** (US 12,403,400 B2, expiry ~2042). Both are limited to phase-based variants.
- **Pending applications:** Cygames (US 2024/0307781 A1, granted in Japan as JP7265607B1, which describes "more flexible motion matching") and Activision's 2026 motion-graph applications.

Ubisoft, Epic Games, and Take-Two had no "motion matching" hits in Google Patents.

### Cited Findings

**Prior public disclosures (dates)**
- Büttner (Ubisoft Toronto) presented "Motion Matching – The Road to Next-Gen Animation" at nucl.ai 2015 — [arXiv 2310.05215 reference list](https://arxiv.org/pdf/2310.05215); [Game Developer](https://www.gamedeveloper.com/programming/most-inspiring-game-animation-tech-talks-of-2016). nucl.ai 2015 ran Monday 20 to Wednesday 22 July 2015, in Vienna — [PocketGamer.biz event listing](https://www.pocketgamer.biz/events/711/nuclai/)
- Motion matching "was first presented by Büttner and Clavet in 2015 as a greedy approximation to the Motion Fields method by Y. Lee et al. from 2010" — [arXiv 2310.05215](https://arxiv.org/pdf/2310.05215)
- GDC 2016 coverage of the Ubisoft talk is dated 15 March 2016 — [DualShockers](https://www.dualshockers.com/ubisoft-introduces-motion-matching-the-first-step-to-next-gen-animation/); Clavet's session: [GDC Vault](https://gdcvault.com/play/1022985/Motion-Matching-and-The-Road)
- EA's later patents cite as prior art "Clavet, 'Motion matching and the road to next-gen animation,' In Proc. of GDC. 2016", Holden's PFNN (2017), Lee et al. Motion Fields, and Starke's Local Motion Phases (2020). Unity's patent cites Kovar "Motion Graphs" (SIGGRAPH 2002), Lee "Motion Fields" (SIGGRAPH Asia 2010), and Holden 2015–2017 — [Google Patents US12403400B2](https://patents.google.com/patent/US12403400B2/en); [US11995754B2](https://patents.google.com/patent/US11995754B2/en); [US10717004B2](https://patents.google.com/patent/US10717004B2/en)

**Granted patents and pending applications**

| Number | Assignee (inventors) | Title | Priority / filed | Granted | Expiry (est.) | Status (at snapshot) |
|---|---|---|---|---|---|---|
| US 10,888,785 B2 | Unity IPR ApS (Michael Büttner) | Method and system for real-time animation generation using machine learning | prio 2018-06-18; filed 2020-06-24 (continuation of 16/444,977) | 2021-01-12 | terminal disclaimer, so no later than parent ~2039-06-18 | presumed active |
| US 10,717,004 B2 | Unity IPR ApS, reassigned to Unity Technologies ApS 2025-09-23 (Büttner) | same title | prio 2018-06-18; filed 2019-06-18 | 2020-07-21 | 2039-06-18 (Google) | Active (Google) |
| EP 3807846 B1 / CA 3104243 C / WO 2019/243899 | Unity | same family | prio 2018-06-18 | EP B1 published 2024-09-11; CA C 2022-08-09 | n/a | listed as granted family members |
| US 11,562,523 B1 | Electronic Arts (W. S. Starke, Y. Zhao, M. Sardari, H. H. Chaput, N. Aghdaie) | Enhanced animation generation based on motion matching using local bone phases | 2021-08-02 | 2023-01-24 | 2041-08-02 (Google) | Active (Google) |
| US 11,995,754 B2 | Electronic Arts (same) | same (continuation) | prio 2021-08-02; filed 2023-01-23 | 2024-05-28 | 2041-08-02 (Google) | Active (Google) |
| US 12,403,400 B2 | Electronic Arts (Starke, Chaput) | Learning character motion alignment with periodic autoencoders | 2022-03-31 | 2025-09-02 | 2042-10-25 adjusted (Google) | Active (Google) |
| US 9,827,496 B1 | Electronic Arts (Fabio Zinno) | System for example-based motion synthesis | prio 2015-03-27; filed 2016-03-25 | 2017-11-28 | 2036-04-18 adjusted (Google) | Active (Google) |
| US 10,388,053 B1 | Electronic Arts (B. F. Carter Jr., F. Zinno) | System for seamless animation transition | prov. 62/139,219 filed 2015-03-27; filed 2016-03-25 | 2019-08-20 | ~2036-06-12 (filing + 20 y + 79 days PTA; my arithmetic) | unknown |
| US 10,022,628 B1 | Electronic Arts (M. Matsumiya, P. Rigiroli) | System for feature-based motion adaptation | prio 2015-03-31; filed 2016-03-25 | 2018-07-17 | 2036-12-31 adjusted (Google) | Active (Google) |
| US 11,836,841 B2 | Tencent (Tianxiang Zhang) | Animation video processing method… | prio CN 2020-02-10; filed 2022-03-04 | 2023-12-05 | 2041-05-24 adjusted (Google) | Active (Google) |
| US 8,947,441 B2 | Disney Enterprises (Jessica Hodgins, Ronit Slyper) | System and method for database driven action capture | prov. 2009-06-05; filed 2010-06-02 | 2015-02-03 | ~2032-08-09 (filing + 20 y + 799 days PTA; my arithmetic) | unknown |
| US 11,055,891 B1 | Microsoft (E. Ofek, M. Gonzalez Franco, A. D. Wilson, K. Ahuja, C. Holz) | Real time styling of motion for virtual environments | filed 2020-03-10 | 2021-07-06 | ~2040-03-10 (0 days PTA; my arithmetic) | unknown |
| US 7,403,202 B1 | Electronic Arts (Shawn P. Nash) | Computer animation of simulated characters using combinations of motion-capture data and external force modelling… | parent filed 2005-07-12 | 2008-07-22 | ~2025-07 (20 y from parent filing; my arithmetic), so very likely expired | likely expired |
| US 2024/0307781 A1 (application) | Cygames (Adan Haefliger) | System, game device, method, and program for determining motion sequence | JP 2021-11-26; PCT/JP2022/042874; US filed 2024-05-23 | pending at publication 2024-09-19 | n/a | US status unknown; JP counterpart JP7265607B1 granted |
| US 2026/0021402 A1, /0021403 A1, /0024263 A1 (applications) | Activision Publishing (A. Bereznyak; M. Farrokhtala on 0021403) | Systems and methods for enabling controlled character motion synthesis / improved character animation stylization / animation of a secondary asset in online multi-player video games | prov. 2024-07-19 (+2024-08-30); filed 2024-10-25 | pending (published 2026-01-22) | n/a | pending |
| US 2021/0405739 A1 / WO 2021/262376 | Sony Interactive Entertainment (Sergey Bashkirov) | Motion matching for VR full body reconstruction | 2020-06-26 | not granted | n/a | **Abandoned** (Google) |

Sources: [Google Patents US10717004B2](https://patents.google.com/patent/US10717004B2/en); [USPTO PDF US10888785](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10888785); [Google Patents US11562523B1](https://patents.google.com/patent/US11562523B1/en); [US11995754B2](https://patents.google.com/patent/US11995754B2/en); [US12403400B2](https://patents.google.com/patent/US12403400B2/en); [USPTO Official Gazette entry US12403400](https://patentsgazette.uspto.gov/week35/OG/html/1538-1/US12403400-20250902.html); [Google Patents US9827496B1](https://patents.google.com/patent/US9827496B1/en); [USPTO PDF US10388053](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10388053); [Google Patents US10022628B1](https://patents.google.com/patent/US10022628B1/en); [Google Patents US11836841B2](https://patents.google.com/patent/US11836841B2/en); [USPTO PDF US8947441](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8947441); [USPTO PDF US11055891](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11055891); [USPTO PDF US7403202](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/7403202); [USPTO PDF US20240307781](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20240307781); [Cygames research, JP7265607B1](https://research.cygames.co.jp/patents/JP7265607B1/); [USPTO PDF US20260021402](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20260021402); [US20260021403](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20260021403); [US20260024263](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20260024263); [Google Patents US20210405739A1](https://patents.google.com/patent/US20210405739A1/en)

**Claim summaries (independent claim 1, paraphrased closely; OCR-derived where noted)**
- **US 10,888,785 B2 (Unity)**, from OCR of the USPTO PDF, claim 1. A system that determines a character pose for a video game frame by:
  1. determining a dominant atom from a set of active atoms, based on weights associated with the active atoms (each atom references a pose in a motion library);
  2. determining a motion fragment for the dominant atom from pose data and future trajectory data in the library;
  3. calculating a predicted future trajectory from the motion fragment and control data from an input device;
  4. determining a combined motion fragment by replacing the fragment's future trajectory with the predicted trajectory;
  5. "searching the motion library for an additional atom that has an associated motion fragment which is most similar to the combined motion fragment";
  6. adding that atom to the active set as a target atom;
  7. posing the character by a weighted combination of the active atoms' poses, and changing the weights.

  The patent carries a terminal disclaimer, and its dependent claims add a trajectory-prediction neural network and collision-avoiding combined trajectories — [USPTO PDF US10888785](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10888785)
- **US 10,717,004 B2 (Unity parent)**, claim 1. Narrower than the continuation. It also requires choosing a "motion controlling ability" by priority and game state, collision detection on the predicted trajectory with a combined trajectory, and "converting the combined motion fragment to a hash value using a hashing function and using a nearest neighbor search" — [Google Patents US10717004B2](https://patents.google.com/patent/US10717004B2/en)
- **US 11,562,523 B1 (EA)**, claim 1. Requires local bone phases representing "contacts of at least one rigid bodies of the in-game character with an in-game environment"; a "local motion matching process for each of the plurality of local bone phases"; selecting local poses; generating a pose from the matched local poses; and rendering. Dependent claims: a 2D phase vector and nearest-neighbour search on it — [Google Patents US11562523B1](https://patents.google.com/patent/US11562523B1/en)
- **US 11,995,754 B2 (EA continuation)**, claim 1. Matches a local bone phase "encoded with local bone phase characteristics including position, orientation, velocity, and acceleration" to local pose animation data, then generates and renders — [Google Patents US11995754B2](https://patents.google.com/patent/US11995754B2/en)
- **US 12,403,400 B2 (EA)**, claim 1. Requires "executing a motion matching process using a motion phase manifold comprising a plurality of local motion phase channels" (each with spatial and temporal data for a character segment). It searches each channel's local pose dataset, scores candidates against reference features, selects local poses, and combines them into a pose — [Google Patents US12403400B2](https://patents.google.com/patent/US12403400B2/en)
- **US 9,827,496 B1 (EA)**, claim 1. Responds to a detected collision: it updates a ragdoll toward a target pose, extracts reference features from the final pose, and identifies a matching motion capture frame — [Google Patents US9827496B1](https://patents.google.com/patent/US9827496B1/en)
- **US 10,388,053 B1 (EA)**, from OCR, claim 1. On an animation change trigger from an in-game event, it selects a new animation using game rules. It determines "a plurality of eligible start frames" from the current frame and an object's context condition, then selects a start frame by comparing reference features of the current frame with each eligible start frame. Dependent claims: eligible frames chosen by tag; features such as joint position, rotation, velocity, and mesh coordinate — [USPTO PDF US10388053](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10388053)
- **US 10,022,628 B1 (EA)**, claim 1. Detects a collision, calculates target features, identifies mocap data from the collision and those features, modifies it to an updated pose, and renders — [Google Patents US10022628B1](https://patents.google.com/patent/US10022628B1/en)
- **US 11,836,841 B2 (Tencent)**, claim 1. Preprocesses an "original animation video" into key frames and motion data. It then determines displacement parameters from a pathfinding movement path with maximum displacement parameters, and outputs a matching animation — [Google Patents US11836841B2](https://patents.google.com/patent/US11836841B2/en)
- **US 8,947,441 B2 (Disney)**, from OCR. Motion-sensor data from a user queries a recorded-motion database by best match, optionally using Haar-wavelet compression ("preserving approximately 10% of the most significant coefficients"). It weights continuity with the prior match and blends between selected motions — [USPTO PDF US8947441](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8947441)
- **US 11,055,891 B1 (Microsoft)**, from OCR, claim 1. Takes real-time virtual reality (VR) user-movement input, extracts features, and compares them to stored motion data. It identifies motions for different body parts, blends them into a stylized motion, maps it into VR space, and drives an avatar — [USPTO PDF US11055891](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11055891)
- **US 2024/0307781 A1 (Cygames, application)**, from OCR, claim 1. "a motion database that stores each of a plurality of motion sequences in association with sub-features of a plurality of types", a sub-feature extraction unit, and a determination unit that picks one motion sequence by comparing sub-features of predetermined types. Dependent claims choose sub-feature types by object state (for example distance to another object) — [USPTO PDF US20240307781](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20240307781). The JP abstract: "A system capable of achieving more flexible motion matching is provided." — [Cygames research, JP7265607B1](https://research.cygames.co.jp/patents/JP7265607B1/)
- **Activision applications (2026)**, from OCR, claim 1 shared core. Generate an offline graph from mocap by identifying "dominant poses" (for example at extrema of a force curve), comparing them pairwise, grouping them into "master pose nodes", and adding transitions between sufficiently similar dominant poses. The variants add GUI stylization with propagation, or secondary-asset inverse blend shapes. The abstract says transitions "that match the plurality of control parameters most closely are selected" at runtime — [USPTO PDF US20260021402](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20260021402); [US20260021403](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20260021403); [US20260024263](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/20260024263)
- **EA US 12,138,543 B1 (generative control; prov. 2020-01-21; filed 2021-01-20; 285 days PTA).** Claim 1 covers an autoencoder over trajectory control information that perturbs user input in latent space and feeds a motion prediction network. This is neural control, not database matching — [USPTO PDF US12138543](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/12138543)

**Search coverage**
- Google Patents phrase queries ("motion matching") restricted by assignee returned: Electronic Arts 5; Sony 41 (almost all video or image processing, plus the abandoned VR application); Activision 3 (the 2026 applications); Disney 1 (US 8,947,441); Microsoft 1 (unrelated HDR video); Roblox 1 application (US 2026/0141609 A1, adapting simulated character interactions); Nvidia 3 (none on database matching); **Ubisoft 0; Epic Games 0; Unity 0; Take-Two 0** — [Google Patents query endpoint, run 2026-10-24](https://patents.google.com/?q=(%22motion+matching%22))
- US-only "motion matching" plus "animation" returned 66 documents. Other game-animation hits beyond those tabulated: NetEase US 2024/0307779 A1 (application), Tencent US 2025/0278881 A1 and US 2025/0356570 A1 (applications), Korea Electronics Technology Institute US 2026/0073644 A1 "Motion matching device and method for heterogeneous 3d models" (application), KAIST US 2026/0154889 A1 (gait), EA US 11,972,353 B2 (motion variational autoencoder (MVAE) controllers), EA US 11,830,121 B1 and US 12,444,115 B2 (neural animation layering), and EA US 12,172,079 B2 (route generation) — [Google Patents query](https://patents.google.com/?q=(%22motion+matching%22)&q=(animation)&country=US)
- No patent on inertialization (Bollo, Gears of War 4, GDC 2018) turned up in searches — [GDC Vault, Inertialization](https://gdcvault.com/play/1025165/Inertialization-High-Performance-Animation-Transitions)

### Inferences
- The Unity continuation (US 10,888,785 B2, and EP3807846B1 in Europe) is the one to show a patent attorney first. Its claim 1 reads on a motion matching runtime that keeps a weighted "blend stack" of active poses, takes a fragment of pose plus future trajectory from the dominant one, swaps in the predicted trajectory, searches for the most similar fragment, pushes it as a new target, and poses from the weighted combination.
  - Two facts may matter to validity: the inventor himself publicly presented motion matching at nucl.ai in July 2015, and priority is 18 June 2018. Only a court or the Patent Trial and Appeal Board can decide whether the 2015–2016 talks anticipate or render obvious these specific elements. It is not settled by these notes.
  - A design that does not pose from a weighted combination of several active database poses is structurally different from that claim element. One example is a single-playing-pose design with inertialization or dead blending, where the transition offset decays rather than blending multiple database poses. That is an engineering observation, not a freedom-to-operate opinion.
- EA's motion matching patents all require local bone or motion phases, a phase manifold, ragdoll and collision triggers, or rule-chosen transition start frames. Classic global-feature motion matching does not use those elements, but a phase-feature extension (for example Starke-style local phases as matching channels) would come closer to US 11,562,523, US 11,995,754, and US 12,403,400.
- The Cygames application claim is broad: sub-features grouped by type, with per-state selection of which types to compare. It resembles schema channels chosen per state (Unreal's channel and database selection). It is pending in the US and granted in Japan. Track it.
- Expired or soon-expiring: patents filed before late 2006 have passed their 20-year term by October 2026 (for example EA US 7,403,202). Motion Graphs (2002) and Motion Fields (2010) are prior art regardless.
- Patents are territorial. US numbers matter for distribution or sale in the US, EP for Europe, JP for Japan. Open-source publication of code is generally treated differently from making or selling a product, but this varies by jurisdiction and needs counsel.

### Gaps
- Google Patents began rate-limiting this session (HTTP 503), so I could not:
  - confirm the current legal status or maintenance-fee payment for US 10,888,785, US 10,388,053, US 8,947,441, and US 11,055,891;
  - check whether Cygames' US application or the Activision applications have been granted since publication;
  - inspect EP3807846B1's claims or opposition history;
  - run assignee searches that do not depend on the phrase "motion matching" for Epic (for example "pose search"), Take-Two and NaturalMotion, Sony (post-2022), or Ubisoft in Canada and France.
- Phrase search misses patents that never say "motion matching". Unity's family did not appear under assignee "Unity" plus "motion matching" and was found only through a broader pose, trajectory, and database query. More such patents may exist.
- Chinese and Korean family members (Tencent, NetEase, KETI) were not analyzed.
- Expiry dates marked "my arithmetic" ignore terminal disclaimers I could not see and any lapse.

---

## 5. Epic EULA terms that restrict porting Unreal Engine source (for example the Pose Search plugin) into another engine

### Takeaway
The Unreal Engine EULA (snapshot 5 October 2026) licenses Engine Code for private use and modification. It lets you distribute Engine Code in source form only to other licensees of the same engine version. Public source distribution must go through Fab or a fork inside Epic's GitHub UnrealEngine network. Public posting is limited to snippets of up to 30 lines, for discussion only, and never "to aggregate, recombine, or reconstruct any larger portion". The EULA bans combining Licensed Technology with copyleft code and bans using it as training input for generative AI. Copying or translating Pose Search plugin code into MIT-licensed Byte-Engine would therefore be a public distribution of Engine Code (or a modified version of it) outside those channels. GASP content (Fab, not Licensed Technology) is separately barred from standalone redistribution.

### Cited Findings
- Scope: "The Licensed Technology … includes all Unreal Engine code and related Epic-provided content … This includes Engine Code, Examples, and Starter Content." Definitions:
  - "'Engine Code' means the Source Code … and object code of Unreal Engine, including any future versions …".
  - "'Source Code' means the human readable form of a software program, including all modules it contains …".
  - "The Licensed Technology includes any modified version of any of the foregoing that you make under the License."
  - Source: [Unreal EULA §1](https://www.unrealengine.com/en-US/eula/unreal) ([Wayback snapshot 2026-10-05](https://web.archive.org/web/20261005183952/https://www.unrealengine.com/en-US/eula/unreal))
- Fab assets are separate: "assets you may download from an Epic-operated online marketplace (including, but not limited to, Fab) are not Licensed Technology and are not licensed to you under the terms of this Agreement." — [Unreal EULA §1](https://www.unrealengine.com/en-US/eula/unreal)
- Grant: "a non-exclusive, non-transferable, non-sublicensable license to privately use, reproduce, display, perform, and modify the Licensed Technology" — [Unreal EULA §2](https://www.unrealengine.com/en-US/eula/unreal)
- Products: "Any Product that you Distribute that incorporates Licensed Technology must incorporate the Licensed Technology only in object code and only as an inseparable part of the Product." "Distribute" includes making code "available publicly" — [Unreal EULA §4](https://www.unrealengine.com/en-US/eula/unreal)
- Source distribution, §5(a)(i): "You may Distribute Engine Code (including as modified by you) in Source Code or object code to a third party who is separately licensed by us to use the same version of the Engine Code". "Any public Distribution of Engine Tools … must take place through a marketplace operated by Epic such as the Fab Marketplace … or through a fork of Epic's GitHub UnrealEngine Network (e.g., for Distributing Source Code)." — [Unreal EULA §5(a)(i)](https://www.unrealengine.com/en-US/eula/unreal)
- Snippets, §5(a)(ii): "You are permitted to post snippets of Engine Code, up to 30 lines of code in length, online in public forums for the sole purpose of discussing the content of the snippet or Distribute such snippets in connection with supporting patches and plug-ins for the Licensed Technology, so long as it is not for the purpose of enabling third parties without a license to the Engine Code to use or modify any Engine Code or to aggregate, recombine, or reconstruct any larger portion of the Engine Code." — [Unreal EULA §5(a)(ii)](https://www.unrealengine.com/en-US/eula/unreal)
- Examples, §5(b): "You may Distribute Examples (including as modified by you) in Source Code or object code to any third party." "Examples" means "content made available by us in the Samples and Templates folders in the install directory." — [Unreal EULA §1, §5(b)](https://www.unrealengine.com/en-US/eula/unreal)
- Copyleft, §6(c): you may not "combine, Distribute, or otherwise use the Licensed Technology with any code or other content which is covered by a license that would directly or indirectly require that all or part of the Licensed Technology be governed under any terms other than those of this Agreement", for example GPL, LGPL (unless merely dynamically linking a shared library), or CC BY-SA — [Unreal EULA §6(c)](https://www.unrealengine.com/en-US/eula/unreal)
- Engine Tools, §6(d): defined as "(i) editors and other tools included in the Engine Code; (ii) any code and modules in either the Developer or Editor folders …; and (iii) other software that may be used to develop standalone products based on the Licensed Technology". Products including them may only be distributed via Fab or the GitHub fork network — [Unreal EULA §6(d)](https://www.unrealengine.com/en-US/eula/unreal)
- General restrictions, §6(e), include: not "misappropriate any of our other products or services"; not "support a claim by you or any third party that the Licensed Technology infringes a patent"; and not "result in using the Licensed Technology as a training input to any Generative AI Program or as prompt-based input where the Generative AI Program trains on input data" — [Unreal EULA §6(e)](https://www.unrealengine.com/en-US/eula/unreal)
- Ownership, §7: "we own all title, ownership rights, and intellectual property rights in the Licensed Technology". Contributions to Epic's GitHub network grant Epic a broad license — [Unreal EULA §7](https://www.unrealengine.com/en-US/eula/unreal)
- An undated older copy of the EULA (hosted on Steam) had the same 30-line snippet clause and the same "same Version" licensee-only source distribution — [Steam-hosted EULA copy](https://store.steampowered.com//eula/818630_eula_0)
- Pose Search is documented as an engine plugin (`Plugins/PoseSearch`) in Epic's engine API reference — [UE API, Plugins/PoseSearch](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch)
- Content side (GASP and other Fab or Epic content): "you may not Distribute Licensed Content in source format to third parties except to employees, affiliates, and contractors". You may not "reverse engineer, decompile, translate, disassemble, or derive source code from Licensed Content", nor sell it "on a 'stand-alone basis'". The agreement also has the NoAI clause — [Epic Content License Agreement §4–§5](https://www.unrealengine.com/en-US/eula/content). The Fab summary adds "You may not: Resell or redistribute the asset for free on a standalone basis" — [Fab Standard License](https://www.fab.com/eula)
- Byte-Engine is MIT-licensed ("MIT License, Copyright (c) 2023 Facundo Villa"; `license = "MIT"` in the workspace `Cargo.toml`) — local repository files `LICENSE` and `Cargo.toml`

### Inferences
- Pose Search code ships with Unreal and is "Source Code … of Unreal Engine, including all modules it contains". Porting or translating it (including to Rust) and publishing it in Byte-Engine's public MIT repository would be a public distribution of Engine Code, or of a modified version, which also counts as Licensed Technology. That falls outside §5's channels and exceeds the 30-line discussion allowance. MIT relicensing would also purport to grant rights Epic does not grant.
- The EULA governs code and content. It does not claim ownership of the algorithm or of ideas presented in public talks. Implementing motion matching from public papers and talks (Clavet 2016, Holden's articles, Lee 2010) is not restricted by the EULA. Patents are a separate question (Section 4).
- To limit copyright and contract risk, contributors who have read the Unreal Pose Search source should avoid transcribing structure or code from it. Write from public documentation and papers instead: a clean-room-style process. Epic's public documentation of features (channels, notify-state names, cost biases) describes behavior, which is generally treated differently from copying code. This is risk management advice, not a legal conclusion.
- GASP animations and Blueprints are Fab or Epic Content, not "Examples" from the Samples folder, so the §5(b) Examples carve-out does not appear to apply. They cannot be redistributed with Byte-Engine.

### Gaps
- I could not reach the live Epic EULA page (HTTP 403) and relied on the 5 October 2026 Wayback snapshot. Later edits would not be reflected.
- I did not find an Epic statement specifically addressing clean-room reimplementations of engine features, or whether Pose Search's source tree includes "Developer" or "Editor" modules (relevant to §6(d) "Engine Tools").
- I did not verify GASP's current per-listing license label inside the Epic Games Launcher or Fab UI (see Section 1 gaps).
