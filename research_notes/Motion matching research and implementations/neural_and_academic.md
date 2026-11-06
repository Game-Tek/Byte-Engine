# Neural and academic lineage of data-driven locomotion around motion matching (to October 2026)

Scope note: these notes cover neural controllers, physics-based controllers, academic extensions of motion matching (MM), industry stance, and step-by-step hybrids. The classic MM algorithm, search acceleration, transitions and inertialization, implementation surveys, and datasets belong to other researchers and appear here only as context. Every paper lists its venue and year. Where a number comes from the authors' own paper, it is the authors' claim, not an independent benchmark.

## 1. Neural real-time controllers: what they take, what they cost, and how they compare with motion matching

### Takeaway
Neural kinematic controllers moved from small per-frame MLPs that run on the CPU in milliseconds (PFNN 2017, MANN 2018, NSM 2019, LMP 2020, Learned Motion Matching 2020) to generative models (Motion VAEs 2020, Codebook Matching 2024) and then to diffusion or latent-generative controllers (A-MDM and CAMDM 2024, DART 2025, MotionBricks 2026). The diffusion controllers are better than MM on diversity and style metrics but cost about 5 to 21 ms per character on a desktop GPU. Only Learned Motion Matching, which deliberately reproduces MM, has a documented production path (Tencent MotorNerve). Most of the Edinburgh/Starke code is under a non-commercial license.

### Cited Findings

**Phase-Functioned Neural Networks (PFNN), Holden, Komura, Saito, SIGGRAPH 2017, ACM TOG 36(4) Art. 42**
- The network's weights come from a cyclic function of the phase, so the phase input selects the weights ([80.lv summary](https://80.lv/articles/phase-functioned-neural-networks-for-character-control)). Trained on "gigabytes of motion data", including rough terrain ([theorangeduck PFNN page](https://theorangeduck.com/page/phase-functioned-neural-networks-character-control)).
- Cost: the abstract claims "a few megabytes of memory" and "only milliseconds of execution time". The page gives no exact per-frame numbers ([theorangeduck PFNN page](https://theorangeduck.com/page/phase-functioned-neural-networks-character-control)).
- Code and data: pfnn.zip (code and data) and a demo download from theorangeduck.com, with no license stated there ([theorangeduck PFNN page](https://theorangeduck.com/page/phase-functioned-neural-networks-character-control)). AI4Animation hosts Unity code and Windows, Linux, and Mac demos. The weights are a separate download ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).

**Mode-Adaptive Neural Networks (MANN), Zhang, Starke (joint first), Komura, Saito, SIGGRAPH 2018, ACM TOG 37(4) Art. 145**
- A motion prediction network plus a gating network. The gate selects and blends "expert weights", and each expert specializes in one movement. It learns from unstructured quadruped mocap without phase labels, covering walk, pace, trot, canter, jump, sit, turn, and idle ([Edinburgh Research Archive](https://era.ed.ac.uk/items/2a0116a0-e9cf-4a36-b0d1-b70c1be532cf)). Described as an extension of PFNN ([NII event page](https://www.nii.ac.jp/en/event/2018/1207.html)).
- Released: paper, code, mocap data, and Windows, Linux, and Mac demos ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).

**Neural State Machine (NSM), Starke and Zhang (joint first), Komura, Saito, SIGGRAPH Asia 2019, ACM TOG 38(6) Art. 178**
- Character–scene interactions (sit, carry, open doors). The repo describes it as "responsive, compact and scalable". Code, demo, and mocap data are released ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).

**Local Motion Phases (LMP), Starke, Zhao, Komura, Zaman, SIGGRAPH 2020, ACM TOG 39(4) Art. 54**
- Multi-contact movements (basketball). Code under `SIGGRAPH_2020` and a Windows demo are released ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).
- In CAMDM's benchmark on 100STYLE, LMP scored FID 1.255, trajectory error 71.7, and style accuracy 51.5%. MM-DP scored 3.455, 53.2, and 23.6% ([CAMDM arXiv 2404.15121](https://arxiv.org/html/2404.15121v1)).

**Neural Animation Layering, Starke, Zhao, Zinno, Komura, SIGGRAPH 2021, ACM TOG 40(4) Art. 92**
- Martial-arts synthesis. Only the paper and video are released, with no code or data ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).

**DeepPhase / Periodic Autoencoder (PAE), Starke, Mason, Komura, SIGGRAPH 2022, ACM TOG 41(4) Art. 136**
- Learns a phase manifold from unstructured motion without supervision, splitting motion into latent periodic channels per body segment. The authors state that distances in the learned features "provide a better similarity measure than in the original motion space". The phase manifold is said to improve "neural character controllers and motion matching", including "motion query for matching poses within large animation databases" ([DeepPhase PDF, HKU](https://i.cs.hku.hk/~taku/deepphase.pdf)). It won a SIGGRAPH 2022 best paper award ([University of Edinburgh](https://informatics.ed.ac.uk/ipab/taku-komura-and-group-win-best-paper-award)).
- Released: PyTorch code and demo, Unity code and demo, an addendum by Ian Mason, and a tutorial. The third-party PhaseBetweener builds on it ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).

**Motion VAEs (MVAE), Ling, Zinno, Cheng, van de Panne, SIGGRAPH 2020 (DOI 10.1145/3386569.3392422, same TOG 39(4) issue as Learned Motion Matching)**
- Autoregressive conditional VAE. Its latent variables "define the action space for the movement", which RL then steers. Code is at github.com/electronicarts/character-motion-vaes ([arXiv 2103.14274](https://arxiv.org/abs/2103.14274)).
- A-MDM reports MVAE's diversity (APD) as 40.97 vs A-MDM's 61.08 on AMASS, and its ADE as 24.42 vs 10.40 cm ([A-MDM arXiv](https://arxiv.org/html/2306.00416v3)).

**Learned Motion Matching (LMM), Holden, Kanoun, Perepichka, Popa (Ubisoft La Forge), SIGGRAPH 2020, ACM TOG 39(4)**
- Three networks replace MM's data lookups:
  - The **Decompressor** maps a feature vector plus autoencoder-extracted extra features to the full pose. Matching features alone give visible errors.
  - The **Stepper** predicts the next frame's combined features. It only needs a short horizon, because MM transitions about five times a second.
  - The **Projector** maps the query directly to combined features, emulating the search.

  Together they remove all databases from memory ([Ubisoft La Forge, Introducing LMM](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)).
- Memory: a large controller (47 extra hand and finger joints, about 30 locomotion styles) takes 590 MB with MM and about 17 MB of weights with LMM. With 16-bit quantization that drops to 8.5 MB, about 70x smaller, with "no visible impact". A small example gives about a 10x improvement ([Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)).
- The stated goal is a "constant CPU cost" that no longer grows with the data. No ms figures appear in the article ([Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)). It preserves MM behavior, so "animators can keep their existing workflow" ([Ubisoft La Forge LMM news](https://www.ubisoft.com/en-us/studio/laforge/news/3NWqwcMU9lMfumgfPAEfka/learned-motion-matching)).
- Limitations stated: each controller needs its own trained set of networks, and the Decompressor is inaccurate without extra features ([Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)).
- Code: [orangeduck/Motion-Matching](https://github.com/orangeduck/Motion-Matching) is MIT-licensed C++ (raylib) with PyTorch training scripts (`train_decompressor.py`, `train_stepper.py`, `train_projector.py`). It exports `.bin` weights loaded by the C++ runtime. Its data comes from the Ubisoft La Forge dataset under CC BY-NC-ND 4.0. The repo leaves out some of the paper's storage optimizations and tags.
- Follow-up: "Learning Robust and Scalable Motion Matching with Lipschitz Continuity and Sparse Mixture of Experts", ACM MIG 2023 (DOI 10.1145/3623264.3624442). It claims 8.5x lower CPU cost than LMM for large character counts, at 80% of LMM's memory, "on contemporary video game hardware". It also says the original LMM cut memory but *increased* performance cost ([search summary of the abstract via unpaywall](https://unpaywall.org/10.1145%2F3623264.3624442)).

**Categorical Codebook Matching, Starke, Starke, He, Komura, Ye, SIGGRAPH 2024, ACM TOG 43(4) Art. 142**
- Maps sparse sensor signals (VR head and hands) to full-body motion in real time. It learns the motion manifold and how to sample from it jointly, end to end, by aligning two categorical codebooks, one for inputs and one for outputs. It addresses ambiguity, works from unstructured mocap, and targets VR and games ([HKU repository](https://hub.hku.hk/handle/10722/362417)).
- Released: code under `SIGGRAPH_2024`, the "Cranberry" mocap dataset, and VR, Windows, and Mac demos ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).
- Starke received the Eurographics Young Researcher Award 2025 ([Eurographics](https://www.eg.org/wp/eurographics-awards-programme/the-young-researcher-award/young-researcher-award-2025-sebastian-starke/)).
- AI4AnimationPy (2026, Paul and Sebastian Starke) is a Python remake under facebookresearch/ai4animationpy, with Hugging Face web demos ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).

**AI4Animation license (covers PFNN Unity, MANN, NSM, LMP, DeepPhase, and Codebook Matching code):** "not freely available for commercial use or redistribution", research and education only. Mocap data is CC BY-NC 4.0 ([AI4Animation README](https://github.com/sebastianstarke/AI4Animation)).

**A-MDM (Interactive Character Control with Auto-Regressive Motion Diffusion Models), Shi, Wang, Jiang, Lin, Dai, Peng, ACM TOG 43(4) 2024 (SIGGRAPH 2024)**
- Per-frame diffusion. The input is the previous state plus a noisy next state, and the state holds root velocity, joint positions, velocities, and 6D rotations. The network is an MLP with 10 layers of 1024 units. Control comes through task-oriented sampling, in-painting, and hierarchical RL ([arXiv 2306.00416](https://arxiv.org/html/2306.00416v3); [NVIDIA](https://research.nvidia.com/publication/_interactive-character-control-auto-regressive-motion-diffusion-models)).
- Cost on an RTX 4090 with an i9-13900K: about 21 ms per frame at 40 steps (about 48 FPS), and about 5.3 to 9 ms at 10 steps. Training data: 100STYLE (over 4M frames), LaFAN1 (over 400k frames), and AMASS. Compared only with MVAE and HuMoR, not with MM ([arXiv 2306.00416](https://arxiv.org/html/2306.00416v3)).

**CAMDM (Taming Diffusion Probabilistic Models for Character Control), Chen, Shi, Huang, Tan, Komura, Chen, SIGGRAPH 2024**
- Transformer autoregressive diffusion:
  - Inputs: 10 past frames, a style label, future root displacements, and orientations.
  - Output: 45 future frames.
  - Real-time techniques: separate condition tokens, classifier-free guidance on past motion, and heuristic trajectory extension.
  - Inference: 8 diffusion steps.

  ([arXiv 2404.15121](https://arxiv.org/html/2404.15121v1))
- Cost: 13 ms of inference at 8 steps, and over 60 FPS on an RTX 3060 "without optimization". Weights are about 20 MB. Training: about 20 h on one A100, on 100STYLE (over 4M frames, 100 styles, 30 FPS) ([arXiv](https://arxiv.org/html/2404.15121v1)). The repo says about one day on full 100STYLE, with usable checkpoints after more than 4 h ([GitHub AIGAnimation/CAMDM](https://github.com/AIGAnimation/CAMDM)).
- Against MM: on single-style control, CAMDM scores FID 0.913 vs MM-DP's 3.455, trajectory error 22.8 vs 53.2, and style accuracy 89.5% vs 23.6%. In multi-style transitions it succeeds 94.2% of the time, vs 56–75.6% for the MM, LMP, and MANN baselines. "MM-DP was evaluated on only 10 styles due to memory constraints" ([arXiv](https://arxiv.org/html/2404.15121v1)).
- Stated limitations: 8 steps are "still computationally demanding" for mobile, the data is flat-ground only, and joystick control has limited expressiveness ([arXiv](https://arxiv.org/html/2404.15121v1)).
- Code: the Unity code is GPL-3 and the rest is Apache-2.0. Models export to ONNX for Unity, and the demo runs on consumer GPUs and Apple Silicon. Inference for arbitrary characters was still "TBA" ([GitHub](https://github.com/AIGAnimation/CAMDM)).
- A later SFU paper (F3AMD, Yin group) argues that Transformer-based systems "are capable of animating one or a few characters in realtime, but remain insufficient for animating large crowds in super-real-time applications such as gaming". It proposes a different noise schedule. Only the search snippet was available, because the PDF fetch failed ([SFU PDF](https://www2.cs.sfu.ca/~kkyin/papers/F3AMD.pdf)).

**DART / DartControl, Zhao et al., ICLR 2025**
- Latent diffusion over motion primitives. The input is 2 history frames plus a CLIP text embedding; each step outputs 8 future frames of SMPL-X. It uses 10 latent diffusion steps, and spatial goals come through optimization or RL ([arXiv 2410.05260](https://arxiv.org/html/2410.05260v3); [mlanthology ICLR 2025](https://mlanthology.org/iclr/2025/zhao2025iclr-dartcontrol)).
- Speed: 334 frames/s on one RTX 4090 for a single sequence (2,394 MiB of memory), about 10x faster than FlowMDM. Trained on BABEL and HML3D (AMASS). Code is at zkf1997.github.io/DART ([arXiv](https://arxiv.org/html/2410.05260v3)).

**MotionBricks, Wang, Dionne, De Ruyter, … Peng, Zhu, Yuen (NVIDIA), ACM TOG / SIGGRAPH 2026 (arXiv 2604.24833, April 2026)**
- One modular latent generative backbone trained on over 350,000 motion clips, plus "smart primitives" that give one interface for navigation and object interaction. Claimed throughput is 15,000 FPS at 2 ms latency, on unspecified hardware. It shows a production-level animation demo and Unitree G1 humanoid deployment, with a project page at nvlabs.github.io/motionbricks ([arXiv 2604.24833](https://arxiv.org/abs/2604.24833)).
- The authors frame real-time interactive control as "still dominated by traditional methods" because generative models lose quality under real-time compute limits and handle multimodal control poorly ([arXiv PDF](https://arxiv.org/pdf/2604.24833)). A UT Austin listing mentions a UE5 production-style demo ([UT RPL](https://rpl.cs.utexas.edu/publications/2026/07/19/wang-siggraph26-motionbricks/)), but the arXiv abstract does not name UE5.

**Control Operators for Interactive Character Animation, Gou, van de Panne, Holden, SIGGRAPH Asia 2025, ACM TOG (DOI 10.1145/3763319), Best Paper Award**
- Encodes diverse control inputs for neural networks in a form that non-technical designers can use to build ML character controllers ([theorangeduck publications](https://www.theorangeduck.com/page/publications)). UBC says it makes it "considerably easier for game designers to develop and train machine-learning models"; earlier neural controllers "required extensive knowledge of machine learning". Gou did the work as an Epic Games intern, and Holden is at Epic Games ([UBC CS news](https://www.cs.ubc.ca/news/2026/02/best-paper-siggraph-asia-2025)).

**Other 2025–2026 neural items seen**
- **STyMo: Fast and Controllable Few-Shot Motion Style Transfer**, Ponton, Winkler, Kavan, Ye, Kadlecek, ACM TOG 45(4) Art. 95, July 2026 (SIGGRAPH 2026). It learns a style from "seconds of paired data" and trains in 1–2 minutes. Runtime controls cover posture intensity, temporal exaggeration, and per-body-region style, and a "stylizability gate" handles out-of-distribution input. The processed paired dataset is released ([arXiv 2609.04500](https://arxiv.org/abs/2609.04500)).
- **Uthana**, "Any Character, Any Movement", SIGGRAPH 2025 Real-Time Live!: a commercial, diffusion-based, real-time motion-control demo ([ACM DL](https://dl.acm.org/doi/10.1145/3721243.3735985), as summarized in search).

### Inferences
- Cost tiers per character:
  - LMM-class MLPs: a few MB and a constant CPU cost.
  - A-MDM, with an MLP and 10 to 40 steps: about 5–21 ms on a 4090.
  - CAMDM, a Transformer with 8 steps: about 13 ms on a 3060.
  - DART: about 3 ms per frame on a 4090.
  - MotionBricks claims 2 ms latency and very high batched throughput, on unspecified hardware.

  For a CPU-side Rust engine animating many characters, only the LMM-class networks fit a per-character budget below 1 ms without a GPU inference path. That is an inference from the cited numbers. No source in this note measured diffusion controllers on a CPU.
- The quality gap that diffusion papers report against MM (FID, style accuracy) is measured against MM-DP baselines that their authors built. MM's memory limit forced the 100STYLE MM-DP baseline down to 10 styles. Read these comparisons as "MM memory scales poorly with many styles", not as "MM looks worse on a curated single-style database".
- Code licenses matter for an engine. AI4Animation (Starke) is non-commercial and cannot be copied into a shipped engine. The orangeduck LMM reference is MIT. CAMDM's non-Unity code is Apache-2.0. Common research datasets (LaFAN1 CC BY-NC-ND, AI4Animation mocap CC BY-NC) restrict commercial use.

### Gaps
- Exact per-frame ms and parameter counts for PFNN, MANN, NSM, LMP, and Codebook Matching were not retrievable; the PDFs exceeded the fetch limit or the pages gave only qualitative claims. Training-data durations (minutes of mocap) for these are also unverified.
- Exact LMM CPU timings (µs per network) are in the paper PDF, which could not be fetched.
- Authors of the MIG 2023 Sparse-MoE LMM paper were not visible in the sources.
- Control Operators' runtime cost, architecture details, and code availability were not found.
- Codebook Matching's runtime cost and whether Meta ships it in Quest avatars are unverified. Meta's leg-tracking reports cite QuestSim, not Codebook Matching ([Mixed News](https://mixed-news.com/en/meta-makes-big-advances-in-quest-full-body-motion-capture/)).
- I found no source showing PFNN, MANN, NSM, LMP, DeepPhase controllers, MVAE, CAMDM, A-MDM, or DART shipping in a commercial game.

## 2. Physics-based controllers as an alternative or complement, and what has shipped

### Takeaway
Physics-based controllers went from per-clip RL imitation (DeepMimic 2018) to style priors (AMP 2021), reusable latent skills (ASE 2022), and unified masked controllers (MaskedMimic 2024). Ubisoft's DReCon (2019) and SuperTrack (2021) are the game-oriented branch, and both use a motion-matching kinematic controller as the reference to track. Training is expensive, about 10 billion samples or 10 GPU-days for ASE. The clearest shipped case by 2026 is Embark's RL-trained physics locomotion for the robot enemies in ARC Raiders (GDC 2026). Humanoid player characters in shipped games are still kinematic.

### Cited Findings
- **DeepMimic**, Peng, Abbeel, Levine, van de Panne, SIGGRAPH 2018, ACM TOG. RL policies imitate mocap or keyframed clips in simulation, combined with task objectives. Demonstrated on a human, Atlas, a dinosaur, and a dragon, including flips and spins ([arXiv 1804.02717](https://arxiv.org/abs/1804.02717v2); [UBC project page](https://www.cs.ubc.ca/~van/papers/2018-TOG-deepMimic/index.html)).
- **AMP (Adversarial Motion Priors)**, Peng, Ma, Abbeel, Levine, Kanazawa, SIGGRAPH 2021, ACM TOG 40(4). A discriminator trained on unstructured clips supplies style rewards, so tracking-based methods' clip-selection machinery is unnecessary. Skill composition "emerges automatically" without a planner ([arXiv 2104.02180](https://arxiv.org/abs/2104.02180v2)).
- **ASE (Adversarial Skill Embeddings)**, Peng, Guo, Halper, Levine, Fidler, SIGGRAPH 2022. The low-level policy took over 10 billion samples (about 10 years of simulated time), about 10 days on one V100, with 4,096 parallel Isaac Gym environments at 120 Hz. The dataset was 187 clips, about 30 minutes ([arXiv 2205.01906](https://arxiv.org/pdf/2205.01906)).
- **MaskedMimic**, Tessler, Guo, Nabati, Chechik, Peng, SIGGRAPH Asia 2024, ACM TOG. One physics controller treated as motion inpainting from partial constraints: keyframes, joint targets, joystick, objects, text, and combinations of these. Trained from tracking data without per-task reward engineering ([NVIDIA](https://research.nvidia.com/labs/par/project/maskedmimic.html); [arXiv 2409.14393](https://arxiv.org/html/2409.14393v1)).
- **DReCon**, Bergamin, Clavet, Holden, Forbes (Ubisoft La Forge / McGill), SIGGRAPH Asia 2019, ACM TOG. An interactive MM kinematic controller, driven by a virtual player, sits inside the RL environment, and the policy learns to track it. It is responsive enough for games, robust to perturbations, and driven by a 10-minute motion database ([Ubisoft Montreal](https://montreal.ubisoft.com/en/drecon-data-driven-responsive-control-of-physics-based-characters/); [theorangeduck publications](https://www.theorangeduck.com/page/publications)).
- **SuperTrack**, Fussell, Bergamin, Holden, SIGGRAPH Asia 2021. A learned world model acts as a differentiable simulator, so the tracking policy trains by supervised backprop instead of PPO. It claims higher quality, shorter training, less sensitivity to dataset size and distribution, and scaling to "much larger databases". No code is released ([theorangeduck SuperTrack](https://theorangeduck.com/page/supertrack-motion-tracking-physically-simulated-characters-using-supervised-learning)). ControlVAE (arXiv 2210.06063) credits it as among the first to track a large diversity of skills ([arXiv 2210.06063](https://arxiv.org/pdf/2210.06063)).
- **Shipped: Embark Studios, ARC Raiders.** GDC 2026 talk "Learning to Move: Physics-Based Enemy Character Locomotion in ARC Raiders" by Martin Singh-Blom (ML Research Lead), 11 March 2026. Reinforcement learning combined with a physics engine replaces "conventional animation playback" for the robot enemies ([Inven Global](https://www.invenglobal.com/articles/20259/nexon-reveals-arc-raiders-reboot-story-at-gdc-2026-after-massive-aaa-reset); [GDC schedule via search](https://schedule.gdconf.com/sessions/?tags%5B%5D=machine_learning_summit)). A secondary blog says tactics stay in behavior trees while the learned model handles movement. This is unconfirmed by Embark ([donweb blog](https://blog.donweb.com/ia-enemigos-arc-raiders-machine-learning/)). No runtime or training costs were published in the coverage ([Inven Global](https://www.invenglobal.com/articles/20259/nexon-reveals-arc-raiders-reboot-story-at-gdc-2026-after-massive-aaa-reset)).
- An earlier Embark GDC ML Summit talk, "Walk Lizzie, Walk! Emergent Physics-Based Animation through Reinforcement Learning" (Jorge del Val Santos), is on GDC Vault. Its year and content could not be verified, because the vault was down ([GDC Vault listing via search](https://www.gdcvault.com/play/1027702/Machine-Learning-Summit-Walk-Lizzie)).
- ARC-RL (University of Florence, arXiv 2605.19503, May 2026) builds MuJoCo environments inspired by ARC Raiders' robots. It explicitly makes no claim about the shipped technique ([alphaXiv](https://www.alphaxiv.org/abs/2605.19503v2)).
- An older precedent: GDC Vault lists "Reinforcement Learning Based Character Locomotion in Hitman: Absolution" ([GDC Vault](https://gdcvault.com/play/1017727/Reinforcement-Learning-Based-Character-Locomotion)).
- Neural physics shipping adjacent to locomotion: LIGHTSPEED STUDIOS' GDC 2026 talk "SoftMax: Neural Simulation of Skeletal Physics" covers secondary physics (hair, cloth, accessories) on dozens of characters in FateTrigger. It claims 0.02–0.13 ms on one CPU core, using quantization-aware training, a custom C++ inference engine, and LOD ([GDC session page](https://schedule.gdconf.com/session/softmax-neural-simulation-of-skeletal-physics/915222)).

### Inferences
- For a game engine, physics controllers are most practical as a complement to MM, not a replacement. DReCon and SuperTrack both use the kinematic MM output as the target the physics character tracks. Building MM first is therefore also the prerequisite for any later physics layer.
- Shipped RL locomotion so far applies to non-human, non-player creatures (ARC Raiders robots), where exact animator-authored style matters less and emergent stumbling is a feature. That fits with player humanoids staying kinematic. This is an inference from the single shipped case found.
- The SoftMax talk shows that studios accept neural inference in shipped games when it runs on the CPU at well under 0.2 ms per character with a custom C++ runtime. That budget is a useful target for any neural component in Byte-Engine.

### Gaps
- No runtime cost, training cost, or architecture details for ARC Raiders' controller were found in public coverage. The GDC Vault was in maintenance.
- EA SEED and Ubisoft La Forge shipped physics-locomotion titles: none confirmed.
- PARC (listed as SIGGRAPH 2025, physics-based augmentation with RL) and GPC (arXiv 2606.29148, "Large-Scale Generative Pretraining for Transferable Motor Control", 2026) appeared only as titles in search results and were not verified.

## 3. Academic extensions of motion matching, 2021–2026

### Takeaway
Academic MM work since 2021 adds new features or costs to the search rather than replacing it:
- predicted body orientation for 3-point VR (Ponton, SCA 2022);
- environment, collision, and height costs for crowds and obstacles (Ponton, SIGGRAPH Asia 2025);
- learned phase manifolds as features, including across morphologies such as dogs (DeepPhase 2022, WalkTheDog 2024);
- per-agent MM in crowd simulators (CGI 2022);
- text and speech front-ends that pick clips through MM (TRiMM 2025).

The UPC-ViRVIG group (Ponton, Pelechano) is the most consistent publisher and releases Unity code.

### Cited Findings
- **Combining Motion Matching and Orientation Prediction to Animate Avatars for Consumer-Grade VR Devices**, Ponton, Yun, Andujar, Pelechano, SCA 2022, Computer Graphics Forum 41(8), pp. 107–118.
  - With only HMD and two controllers, a neural network predicts body orientation. That prediction, plus HMD velocity and rotation, forms the MM feature vector. The result is varied lower-body animation that matches the user's orientation, including stepping in any direction.
  - The VR mocap data is on Zenodo, and the Unity code is at [UPC-ViRVIG/MMVR](https://github.com/UPC-ViRVIG/MMVR) ([EG Digital Library](https://diglib.eg.org/items/217984e0-450a-4d1d-8dba-c9b701ea88a8/full); [arXiv 2209.11478](https://export.arxiv.org/abs/2209.11478)).
- **Environment-aware Motion Matching**, Ponton, Andrews, Andujar, Pelechano, SIGGRAPH Asia 2025, ACM TOG (arXiv 2510.22632).
  - Preprocessing extracts shape, pose, and trajectory features. At runtime, the search penalizes collisions with dynamic obstacles and other agents, so pose and trajectory adapt together in crowded scenes. Height features handle vertical obstacles such as fence jumps, and the demos include corridors, moving obstacles, and props ([arXiv](https://arxiv.org/abs/2510.22632); [project page](https://upc-virvig.github.io/Environment-aware-Motion-Matching/)).
  - The authors argue that crowd systems usually separate trajectory planning from body animation, and that MM struggles with environment interactions and crowds ([project page](https://upc-virvig.github.io/Environment-aware-Motion-Matching/)).
  - Code: [UPC-ViRVIG/Environment-aware-Motion-Matching](https://github.com/UPC-ViRVIG/Environment-aware-Motion-Matching). No license or timing numbers were given on the project page.
- **Crowd Simulation with Detailed Body Motion and Interaction**, Yao et al., CGI 2022. Each agent's animation system switches between a finite state machine and motion matching, so body motion adapts during interactions with agents and the environment. This comes from a search-engine summary; the hosting URL it cited could not be matched to this exact paper, so verify before quoting ([search result: UCL Discovery](https://discovery.ucl.ac.uk/id/eprint/10215224); [possible HKU record](https://hub.hku.hk/handle/10722/288978)).
- **WalkTheDog: Cross-Morphology Motion Alignment via Phase Manifolds**, Li, Starke, Ye, Sorkine-Hornung, SIGGRAPH 2024 Conference Papers. A vector-quantized periodic autoencoder learns one phase manifold shared across skeletons (human and dog). Combined with "an improved motion matching framework", it supports retrieval, transfer, and stylization. Code and pretrained models are at peizhuoli.github.io/walkthedog ([arXiv 2407.18946](https://arxiv.org/pdf/2407.18946)).
- **DeepPhase** (SIGGRAPH 2022): learned phase features improve similarity for MM pose queries in large databases ([DeepPhase PDF](https://i.cs.hku.hk/~taku/deepphase.pdf)). See section 1.
- **Quadruped**: MANN (SIGGRAPH 2018) is the neural quadruped reference ([AI4Animation](https://github.com/sebastianstarke/AI4Animation)). WalkTheDog (2024) is the MM-adjacent quadruped work ([arXiv 2407.18946](https://arxiv.org/pdf/2407.18946)).
- **Text and multimodal hybrids**:
  - **TRiMM**, arXiv 2506.01077, June 2025: a Transformer over text, speech, and motion features runs real-time multimodal inference, then selects clips from a large library through motion matching and streams them to Unreal Engine over LiveLink. It frames classic MM as limited in action diversity ([arXiv 2506.01077](https://arxiv.org/pdf/2506.01077)).
  - **Story-to-Motion**, arXiv 2311.07446, 2023: long-text character animation under trajectory constraints ([arXiv](https://arxiv.com/abs/2311.07446)).
  - DART (ICLR 2025) is the main real-time text-driven neural alternative; see section 1.
- **Example-based generative MM**: "Example-based Motion Synthesis via Generative Motion Matching" (GenMM), [arXiv 2306.00378](https://arxiv.org/pdf/2306.00378). It generates motion from a few examples without training.
- **Other MM uses** cited in a 2025 paper's related work: controllable gesture synthesis from speech with MM (Habibie et al., 2022), and locomotion authoring by footstep and gait-cycle matching (Kim et al., 2024) ([arXiv 2512.12664, as summarized by search](https://arxiv.org/abs/2512.12664v1)).
- **Stylized MM**: STyMo (SIGGRAPH 2026) is few-shot neural style transfer that could post-process any motion stream, but its abstract does not mention MM ([arXiv 2609.04500](https://arxiv.org/abs/2609.04500)). WalkTheDog lists stylization through its MM framework ([arXiv 2407.18946](https://arxiv.org/pdf/2407.18946)).

### Inferences
- The academic pattern that suits an engine is to keep the MM search and add feature channels or cost terms. Examples are predicted orientation, environment penalties, height, and learned phase. Each of these is a separable layer: offline feature extraction, a runtime cost function, and the search. This matches Byte-Engine's "separate concerns by layers" rule.
- Environment-aware MM is the closest academic fit to Byte-Engine's current navigation work (crowds, steering). Its pose-and-trajectory coupling through collision costs in the MM search could replace part of the steering-to-animation hand-off.

### Gaps
- No interaction-aware (two-character, close-contact) MM paper from 2021–2026 was found at SCA, MIG, or Eurographics in these searches. NSM (2019) and MaskedMimic (2024) cover interactions with neural or physics methods instead.
- No dedicated quadruped motion matching paper was found beyond WalkTheDog.
- GenMM's venue (reported elsewhere as SIGGRAPH 2023) and the venues of Kim et al. 2024 and Story-to-Motion were not verified.
- Runtime numbers (ms per agent, agent counts) for Environment-aware MM were not on the project page.
- A systematic sweep of SCA, MIG, and Eurographics 2021–2026 proceedings for "motion matching" was not possible with web search alone. Some smaller MM papers are likely missing.

## 4. Industry stance as of 2026: why shipped games still use classic motion matching, and where neural methods ship

### Takeaway
Industry and academic sources agree on why classic MM dominates: flexibility, predictability, low preprocessing time, visual quality, and an animator workflow that needs no graphs or hand-cut transitions. Its known weakness is memory that grows linearly with data. Neural methods ship in narrow forms: LMM-style compression (Tencent MotorNerve), proprietary ML in sports titles (EA HyperMotion), RL physics for non-human enemies (ARC Raiders), and neural secondary physics (FateTrigger). Even NVIDIA's 2026 MotionBricks paper says real-time interactive control is "still dominated by traditional methods".

### Cited Findings
- The LMM paper states that MM "remain[s] a popular choice in the games industry due to their flexibility, predictability, low preprocessing time, and visual quality" ([search summary of LMM abstract](https://www.theorangeduck.com/page/learned-motion-matching); [SIGGRAPH history](https://history.siggraph.org/?p=78733)).
- Ubisoft La Forge on MM's strengths: "simple yet powerful". It needs little manual work after setup, and animators need not build graphs, cut or synchronize clips, or author transitions. It adapts to props, rough terrain, and reactions. Its weaknesses: memory grows with data and features, search slows with database size, it cannot combine separate actions (walking while drinking), and trajectory features trade continuity for path adherence ([Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)).
- MotionBricks (SIGGRAPH 2026) says real-time interactive motion control is still dominated by traditional methods, because generative models lose quality or scale under real-time compute and handle multimodal control poorly ([arXiv PDF 2604.24833](https://arxiv.org/pdf/2604.24833)).
- CAMDM's authors call their 8-step model "still computationally demanding", which limits mobile use ([arXiv 2404.15121](https://arxiv.org/html/2404.15121v1)). The F3AMD authors say Transformer diffusion is insufficient for large crowds in games ([SFU PDF snippet](https://www2.cs.sfu.ca/~kkyin/papers/F3AMD.pdf)).
- Control Operators (Epic/UBC, SIGGRAPH Asia 2025) is motivated by neural controllers having "required extensive knowledge of machine learning" to customize ([UBC](https://www.cs.ubc.ca/news/2026/02/best-paper-siggraph-asia-2025)). That is a designer-control barrier to adoption, named by an engine vendor's research team.
- **Tencent MotorNerve** (TiMi Studio Group), GDC 2024, by Yuchen Liao and Songnan Li. Locomotion "combines Motion Matching with Learned Motion Matching" to achieve "high-quality, low-consumption locomotion control" and to speed up LMM tuning. Transitions use an in-house VAE-based in-betweening method (Zhejiang University with TiMi, SIGGRAPH 2022) that reduces foot skating ([GDC 2024 schedule](https://schedule.gdconf.com/session/motornervea-character-animation-system-using-machine-learning-presented-by-tencent-games/903481); [TechNode](https://technode.global/?p=77055)).
- **EA HyperMotion** (FIFA 22, 2021): a "proprietary machine-learning algorithm learns from over 8.7 million frames of advanced match capture, then writes new animations in real-time" ([TechRadar](https://www.techradar.com/news/hypermotion-technology-in-fifa-22-sounds-like-a-game-changer-but-what-is-it); [80.lv](https://80.lv/articles/ea-uses-machine-learning-for-animations-in-fifa-22)). EA told Sports Gamers Online in 2021 that it had *not* implemented Starke's prototype neural engine in EA Sports games ([SGO](https://www.sportsgamersonline.com/games/soccer/hypermotions-mma-future-with-ea-sports/amp/)). EA Sports FC 24 used HyperMotionV, built on volumetric data from 180 matches ([Wikipedia FC 24](https://wikipedia.com/wiki/FC_24)).
- **Embark ARC Raiders**: RL physics locomotion for enemies, presented at GDC 2026 ([Inven Global](https://www.invenglobal.com/articles/20259/nexon-reveals-arc-raiders-reboot-story-at-gdc-2026-after-massive-aaa-reset)).
- **LIGHTSPEED STUDIOS FateTrigger**: neural skeletal secondary physics in production, 0.02–0.13 ms per CPU core ([GDC 2026 session](https://schedule.gdconf.com/session/softmax-neural-simulation-of-skeletal-physics/915222)).
- Researcher movement: Starke's GDC talk "4 Years of Bringing Characters to Life with Computer Brains" lists him at EA and Edinburgh ([GDC Vault](https://gdcvault.com/play/1027978/Machine-Learning-Summit-4-Years)). His 2024–2026 work is with Meta collaborators (Yuting Ye; facebookresearch/ai4animationpy) ([AI4Animation](https://github.com/sebastianstarke/AI4Animation)). Holden moved from Ubisoft La Forge to Epic Games ([UBC](https://www.cs.ubc.ca/news/2026/02/best-paper-siggraph-asia-2025)). Epic also ran a GDC 2024 "Unreal Engine 5.4 Animation Deep Dive" ([GDC schedule](https://schedule.gdconf.com/session/unreal-engine-54-animation-deep-dive-presented-by-epic-games/904103)); its MM content is covered by other researchers.

### Inferences
- Reasons for MM's dominance supported by the sources: predictability and debuggability (an output is always a real authored frame), artist workflow (drop clips in, no training loop), low preprocessing, and CPU-only cost. Determinism is implied by an exact nearest-neighbour search over fixed data but was not stated explicitly by a source. Memory is MM's main cost, and LMM addresses it while keeping the behavior.
- Neural methods ship where they either reproduce MM (LMM in MotorNerve), work offline or in proprietary pipelines with huge first-party data (EA), target creatures where emergent motion is wanted (ARC Raiders), or replace a known-expensive simulation with a tiny CPU network (SoftMax). Diffusion controllers have no confirmed shipped title as of October 2026 in these sources.

### Gaps
- Which shipped Tencent titles use MotorNerve was not stated.
- Whether any Ubisoft title shipped LMM was not found.
- No 2025–2026 primary-source survey of studio adoption (talks or interviews explaining why studios avoid neural locomotion) was found. The reasons above come from paper abstracts and vendor blogs.
- Determinism and replay or netcode concerns about neural controllers are commonly cited informally, but no source was found that states them.

## 5. Hybrids an engine could adopt step by step

### Takeaway
The literature supports a staged path that keeps classic MM as the source of truth and adds learned parts one at a time, each separately testable:
1. Learned feature channels (DeepPhase or WalkTheDog phases; predicted orientation as in MMVR) inside the classic search.
2. Learned trajectory or intent prediction feeding the query.
3. LMM compression (Decompressor, then Stepper, then Projector) once memory matters.
4. Neural post-processing (style transfer, physics tracking such as DReCon or SuperTrack).

Each stage has published precedent and, except phase features, an MIT or Apache reference implementation or a shipped example.

### Cited Findings
- **Learned features inside MM**: DeepPhase phase-manifold distances are a better pose similarity measure for MM queries ([DeepPhase PDF](https://i.cs.hku.hk/~taku/deepphase.pdf)). WalkTheDog's VQ phase manifold plugs into an "improved motion matching framework" across morphologies ([arXiv 2407.18946](https://arxiv.org/pdf/2407.18946)). MMVR feeds a network-predicted body orientation into the MM feature vector ([EG Digital Library](https://diglib.eg.org/items/217984e0-450a-4d1d-8dba-c9b701ea88a8/full)).
- **Learned query front-ends**: TRiMM uses a Transformer to turn text and speech into MM queries over a clip library ([arXiv 2506.01077](https://arxiv.org/pdf/2506.01077)). MMVR predicts orientation from sparse VR signals ([GitHub MMVR](https://github.com/UPC-ViRVIG/MMVR)).
- **LMM as compression**:
  - The staged design: the Decompressor alone replaces pose storage, the Stepper removes per-frame lookups, and the Projector removes the search database ([Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)).
  - The cost: 590 MB down to 17 MB, or 8.5 MB quantized ([Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)).
  - MotorNerve shipped MM and LMM combined ([GDC 2024](https://schedule.gdconf.com/session/motornervea-character-animation-system-using-machine-learning-presented-by-tencent-games/903481)).
  - The Sparse-MoE plus Lipschitz variant cuts CPU cost 8.5x at large character counts ([MIG 2023 via unpaywall](https://unpaywall.org/10.1145%2F3623264.3624442)).
  - The reference implementation is MIT C++ with PyTorch training ([orangeduck/Motion-Matching](https://github.com/orangeduck/Motion-Matching)).
- **Physics on top of MM**: DReCon trains an RL policy to track an MM controller ([Ubisoft Montreal](https://montreal.ubisoft.com/en/drecon-data-driven-responsive-control-of-physics-based-characters/)). SuperTrack trains the tracker with a learned world model and supervised learning, with shorter training than PPO ([theorangeduck](https://theorangeduck.com/page/supertrack-motion-tracking-physically-simulated-characters-using-supervised-learning)).
- **Neural post-processing**: STyMo learns styles from seconds of paired data in 1–2 minutes of training, with runtime intensity controls ([arXiv 2609.04500](https://arxiv.org/abs/2609.04500)). MotorNerve uses learned in-betweening for transitions ([GDC 2024](https://schedule.gdconf.com/session/motornervea-character-animation-system-using-machine-learning-presented-by-tencent-games/903481)).
- **Full neural replacement** remains the costly end of the path: CAMDM at 13 ms on a GPU with 20 MB of weights ([arXiv](https://arxiv.org/html/2404.15121v1)), A-MDM at 5–21 ms on a 4090 ([arXiv](https://arxiv.org/html/2306.00416v3)), and MotionBricks at 2 ms on unspecified hardware with 350k training clips ([arXiv](https://arxiv.org/abs/2604.24833)).

### Inferences
- Recommended order for Byte-Engine, inferred from the cited costs and from AGENTS.md's layering and allocation rules:
  1. **Classic MM** with a feature schema that has room for extra channels. It is deterministic, CPU-only, needs no ML runtime, and ships in every engine.
  2. **Offline-learned feature channels** (phase, predicted orientation). Runtime stays a classic search; only the bake step changes.
  3. **A small learned trajectory or intent predictor** as an optional query source.
  4. **LMM Decompressor, Stepper, and Projector** when the database exceeds a memory budget. These are plain MLPs: the orangeduck runtime loads `.bin` weights into C++, which suggests that a few dense matrix multiplies in Rust suffice with no ML framework.
  5. **Optional physics tracking** (DReCon or SuperTrack style) for reactive characters.
- Diffusion or Transformer controllers fit only if Byte-Engine adds a GPU inference path and accepts per-character GPU milliseconds. They also need large datasets (100STYLE at about 4M frames; MotionBricks at 350k clips) whose common public versions are non-commercial or unclear in license.
- Each hybrid stage keeps a classic-MM fallback path. That supports black-box testing against the reference docs: the LMM output should match MM output within a tolerance, which is the property LMM was designed to keep.

### Gaps
- No source gave a Rust-specific ML inference benchmark for these networks.
- No public numbers give LMM per-network CPU µs. The paper PDF could not be fetched.
- Whether learned phase features have been adopted in any shipped MM system is unknown.
- Licensing for 100STYLE and other training sets was not checked here; the datasets researcher covers this.
