# Motion matching runtime engineering: search acceleration, memory layout, and performance at scale

Scope: CPU-side runtime and memory of the motion matching nearest-neighbor pose search, with reported numbers. GPU only where it is used in practice. Neural methods appear only as a memory-compression technique (Learned Motion Matching). State of the art as of October 2026. Crate metadata was read from the crates.io API on 2026-10-24.

Notation used below: N = searchable frames (poses), D = feature dimensions (floats per pose). Brute-force cost is O(N x D) per search, per character.

## 1. Brute-force cost versus database size, and reported database sizes, search times, and character counts

### Takeaway
Typical public feature vectors are small (27 floats in Holden's reference and in every Unity and GPU derivative; 59 columns in O3DE's default), and public databases range from about 15,000 poses (LAFAN1) to about 100,000 to 400,000 poses. A tuned linear scan over about 15,000 to 28,000 poses x 27 dimensions costs about 0.2 to 0.9 ms on one core; Holden's two-level bounding-box pruning brings the same searches to about 0.03 to 0.09 ms, and about 0.22 ms for about 100,000 poses. Shipped games publish almost no per-search timings; For Honor reports "over 100 characters" with aggressive LOD.

### Cited Findings
**Feature vector sizes**
- Holden's reference implementation uses a 27-float feature vector: left and right foot position (3 + 3), left and right foot velocity (3 + 3), hip velocity (3), trajectory positions at +20/+40/+60 frames (X/Z, 6), and trajectory directions at the same frames (X/Z, 6). Positions and velocities are in the root's local frame. — [orangeduck/Motion-Matching database.h](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/database.h)
- The Unity thesis by Ponton et al. uses the same 27-dimensional layout: feet and hip velocities (3x3), feet positions (3x2), future 2D trajectory positions (2x3) and directions (2x3) at 0.33, 0.66, and 1.00 s, normalized by mean and standard deviation. — [Motion Matching for Character Animation and Virtual Reality Avatars in Unity (arXiv 2310.05215)](https://arxiv.org/html/2310.05215)
- Traverse Research's GPU implementation also uses 27 floats per frame (15 from the database row being played, 12 from the controller's predicted trajectory). — [Traverse: Motion matching on the GPU](https://traverse.nl/blog/motion-matching-on-the-gpu)
- Environment-aware Motion Matching (2025) uses 27 query dimensions plus 9 environment dimensions, for a 36-dimensional feature vector; the stored pose vector is 4J+3+2 = 101 values for J = 24 joints. — [Environment-aware Motion Matching (arXiv 2510.22632)](https://arxiv.org/html/2510.22632)
- O3DE's default schema: 59 features (columns) over 1 hour of animation at 30 Hz (108,000 frames), about 6.4 million values using about 24.3 MB. — [O3DE blog: Motion Matching in O3DE](https://docs.o3de.org/blog/posts/blog-motionmatching/)

**Database sizes**
- Unity thesis databases: custom Xsens 18,120 poses; Ubisoft LAFAN1 14,867 poses; custom VR avatar 28,271 poses. — [arXiv 2310.05215](https://arxiv.org/html/2310.05215)
- Environment-aware Motion Matching: reference database 101,026 poses (the code and data section lists 184,553 poses, about 50 min, and the paper does not reconcile the two); scalability tests at 26,455 / 48,575 / 75,341 / 154,214 / 209,152 / 301,794 / 411,280 poses; 60 Hz data; about 50 MB uncompressed for mocap plus extracted features. — [arXiv 2510.22632](https://arxiv.org/html/2510.22632)
- Traverse's early estimate of its database was "a few hundred thousand frames"; matching window 0.333 s = 20 frames at 60 Hz. — [Traverse](https://traverse.nl/blog/motion-matching-on-the-gpu)
- Ubisoft La Forge's large Learned Motion Matching test controller (47 extra hand and finger joints, about 30 locomotion styles) needed "over half a gigabyte" (590 MB) under classic motion matching. — [Ubisoft La Forge: Introducing Learned Motion Matching](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)
- Community documentation for the MxM Unity asset says its Anim Data assets range from 50 MB to 500+ MB. This is a community-fork documentation site, not a benchmark. — [Motion Matching for Unity user manual](https://motion-matching-for-unity.readthedocs.io/en/latest/user-manual/)

**Search times (single character, single search)**
- Unity thesis, Table 3.1, averaged over 100 searches (ms):

  | Database | Linear (Mono) | Linear + Burst | Linear manually vectorized + Burst | BVH (Mono) | BVH + Burst |
  |---|---|---|---|---|---|
  | Xsens (18,120 poses) | 17.36 | 0.45 | 0.21 | 0.31 | 0.03 |
  | LAFAN1 (14,867 poses) | 14.26 | 0.84 | 0.48 | 0.60 | 0.07 |
  | VR (28,271 poses) | 27.07 | 1.58 | 0.91 | 0.39 | 0.05 |

  The "BVH" is Holden's two-level AABB scheme over groups of 64 and 16 consecutive frames. The extracted column headers were garbled, and the alignment above follows the methods the thesis describes. — [arXiv 2310.05215](https://arxiv.org/html/2310.05215)
- The same thesis reports that Holden et al.'s C++ implementation searched LAFAN1 in an average of 0.09 ms. — [arXiv 2310.05215 (PDF)](https://arxiv.org/pdf/2310.05215)
- Environment-aware Motion Matching on an Intel Core i7-12700K, single thread, Burst-compiled Unity: base motion matching with the BVH 0.22 ± 0.05 ms per search on the about-100k-pose database; a linear search with environment terms 9.50 ± 4.16 ms; their full environment-aware search 0.80 ± 0.16 ms in a worst-case dense-obstacle test. With BVH, search cost "grows with a reduced slope" as the database grows (exact values only in Figure 11), and the standard deviation grows in proportion to database size, so the authors suggest a budget-capped search. — [arXiv 2510.22632](https://arxiv.org/html/2510.22632)

**Character counts**
- For Honor (Simon Clavet, GDC 2016): the system ran efficiently with "over 100 characters on screen simultaneously" by using aggressive level of detail so distant characters are not updated every frame. The write-up calls the selection a "ridiculously brute-force approach." — [Game Anim: Motion-Matching in Ubisoft's For Honor](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/); talk listing at [GDC Vault](https://gdcvault.com/play/1022985/Motion-Matching-and-The-Road)
- Unity thesis estimate (not a measurement): at about 0.03 ms per search, run every ten frames, inside a 1 ms per-frame animation budget, "we could animate more than 300 virtual characters simultaneously." It also states that modern productions have around a 1 ms CPU budget for all animation in a scene. — [arXiv 2310.05215](https://arxiv.org/html/2310.05215)
- Traverse GPU: a 250-character debug capture with exhaustive search on the GPU (see question 6). — [Traverse](https://traverse.nl/blog/motion-matching-on-the-gpu)

### Inferences
- Per-pose cost of a tuned scalar or auto-vectorized linear scan in the thesis is about 0.48 ms / 14,867 ≈ 32 ns per 27-float pose, or about 1.2 ns per dimension (LAFAN1, manually vectorized + Burst). Burst alone gave about a 17x to 39x speedup over Mono, and the AABB scheme a further about 7x to 18x over the best linear scan.
- Raw feature memory is N x D x 4 bytes in f32: 15k x 27 ≈ 1.6 MB (fits in L2 on many CPUs); 100k x 27 ≈ 10.8 MB; O3DE's 108k x 59 ≈ 25.5 MB (reported as 24.3 MB, consistent with MiB). Once the feature matrix exceeds the last-level cache share, a brute-force scan becomes memory-bandwidth bound, and every searching character streams the whole matrix.
- Search cost multiplies by the number of searching characters and by search frequency, so per-frame cost ≈ characters x (searches per second / frame rate) x per-search cost. Search frequency (every 5 to 10 frames is common) is the cheapest lever before acceleration structures.

### Gaps
- No public per-search ms figures were found for For Honor, The Last of Us Part II (GDC 2021 talk by Michal Mach and Maksym Zhuravlov is listed by [Naughty Dog](https://www.naughtydog.com/blog/naughty_dog_at_gdc_2021) but its content was not accessible), or any Unreal Engine shipped title. The GDC Vault slides are login-gated.
- The exact Figure 11 scaling numbers of Environment-aware Motion Matching were not extracted (figure only).
- A search summary surfaced an unattributed blog claim that a well-optimized search costs 0.1 to 0.5 ms per character; no primary source was found, so it is not used above.

## 2. Acceleration structures, early-out pruning, partitioning, and the continuing-pose check

### Takeaway
The proven CPU technique is exact branch-and-bound over time-contiguous blocks: Holden's two-level AABB scheme (64-frame and 16-frame boxes, min and max per dimension) plus a partial-distance early-out, seeded with the current pose's cost. Engines that use trees (Unreal PCA + KD-tree, O3DE KD-tree, MxM KD-tree) treat them as approximate broad phases followed by a full-cost re-rank of a candidate set. VP-trees, HNSW, product quantization, and LSH have no documented motion-matching production use except Unreal's experimental VP-tree and Kinematica's per-float codebooks. Bounding-box pruning tolerates runtime per-dimension weights structurally; PCA, VP-trees, and graph indexes are built for one fixed metric.

### Cited Findings
**Holden's two-level AABB scheme (orangeduck/Motion-Matching, `database.h`)**
- Constants: `BOUND_SM_SIZE = 16`, `BOUND_LR_SIZE = 64`. Four `array2d<float>` tables hold per-block, per-dimension `min` and `max` for small and large blocks; block counts use ceiling division; frame i maps to block i/16 and i/64. — [database.h](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/database.h)
- Search loop: iterates clip ranges, stopping `ignore_range_end` (default 20) frames before each range end. If a current index is given, its cost is computed first and used as the initial pruning threshold. For each 64-frame box it computes the query-to-box squared distance starting from `transition_cost`; if that lower bound is ≥ `best_cost`, it skips the box. Surviving boxes repeat the test on their four 16-frame boxes. Inside surviving small boxes, frames within `ignore_surrounding` (default 20) of the current frame are skipped, and the per-frame accumulation breaks as soon as the partial sum reaches `best_cost`. `transition_cost` is added to bounds and frame costs alike, so the bounds stay valid lower bounds; the pruning is exact apart from the deliberate exclusions. — [database.h](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/database.h)
- Weights are baked at build time: per-group mean offset, one shared standard deviation per feature group, and `features_scale = std / weight`; queries apply `(query - offset) / scale`. Changing weights requires regenerating the database (and retraining LMM networks). — [database.h](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/database.h); [README](https://github.com/orangeduck/Motion-Matching)
- The README notes the demo omits tags that separate walking and running, which the LMM paper used. — [orangeduck/Motion-Matching README](https://github.com/orangeduck/Motion-Matching)
- The Environment-aware paper describes standard motion matching with a BVH as scaling "logarithmically" with database size; its environment-aware cost requires a linear traversal, so it falls back to the BVH only when no obstacle is near any ellipse. It also adds temporal coherence: minimum stride of 8 frames, a sparse set of representative indices at least 8 frames apart and at least 5% different, an adaptive stride with aggressiveness v = 5, and starting the scan about 1% before the previous best index. — [arXiv 2510.22632](https://arxiv.org/html/2510.22632)

**KD-trees**
- Unreal Pose Search: `PCAKDTree` projects poses into PCA space with the most significant `NumberOfPrincipalComponents` dimensions and builds a KD-tree; KD-tree results "will have only an approximate cost," so the search re-ranks the best `KDTreeQueryNumNeighbors` poses with the full cost. Details in question 3. — [UE Python API: PoseSearchMode (5.3)](https://docs.unrealengine.com/5.3/en-US/PythonAPI/class/PoseSearchMode.html); [UE Python API: PoseSearchDatabase (5.3)](https://docs.unrealengine.com/5.3/en-US/PythonAPI/class/PoseSearchDatabase.html)
- O3DE: a KD-tree broad phase returns one leaf bucket of candidate frames with no backtracking, then a brute-force narrow phase scores those candidates. Details in question 4. — [O3DE KdTree.cpp](https://raw.githubusercontent.com/o3de/o3de/development/Gems/MotionMatching/Code/Source/KdTree.cpp)
- MxM (Unity asset) builds a KD-tree acceleration structure at preprocessing and searches with Burst-compiled jobs (community documentation). — [Motion Matching for Unity user manual](https://motion-matching-for-unity.readthedocs.io/en/latest/user-manual/)
- Krüger et al. (SCA 2010) showed medium-dimensional feature sets (15 to 90 dimensions) are practical for KD-tree nearest-neighbor searches over very large motion databases (CMU + HDM05, over 750 min). — [Eurographics Digital Library](https://diglib.eg.org/items/cdf26b10-36d2-416d-afeb-24173d289836/full). A summary of the paper reports approximate KD-tree search with a small epsilon gave the same results as exact search while cutting search time by about 10%, and epsilon = 0.5 lowered accuracy noticeably. — [ResearchGate entry](https://www.researchgate.net/publication/312771174_Fast_local_and_global_similarity_searches_in_large_motion_capture_databases) (secondary summary)
- General ANN background: the KD-tree works well for exact nearest-neighbor search in low dimensions but loses effectiveness as dimensionality increases; multiple randomized KD-trees often give about an order-of-magnitude gain over a single KD-tree (FLANN). — [Muja and Lowe 2009](https://www.cs.ubc.ca/~lowe/papers/09muja.pdf)

**VP-trees, ball trees, HNSW, PQ, LSH**
- Unreal offers `VPTree` mode, marked "Experimental, this feature might be removed without warning, not for production use" (UE 5.8 docs). — [EPoseSearchMode](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/EPoseSearchMode)
- VP-trees require a true metric: the `vpsearch` crate warns "This algorithm does not work with squared distances." — [vpsearch docs](https://docs.rs/vpsearch/latest/vpsearch/)
- Holden's projector training script uses a scikit-learn `BallTree` over all features to compute nearest-neighbor ground truth offline (training, not runtime). — [train_projector.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/train_projector.py)
- Unity Kinematica (preview, 2020) encodes each float of each "fragment" as a `kNumCodeBits`-bit code that indexes the closest centroid, in per-tag codebooks that "perform fast comparisons for poses and trajectories"; poses and trajectories compare separately. The docs do not call it product quantization. — [Kinematica CodeBook API](https://docs.unity.cn/Packages/com.unity.kinematica@0.8/api/Unity.Kinematica.Binary.CodeBook.html); [Kinematica Asset Builder](https://docs.unity3d.com/Packages/com.unity.kinematica@0.8/manual/Builder.html)
- No source found that uses HNSW, FAISS, or LSH for runtime motion matching. (Searches for motion-matching-specific ANN comparisons returned only general ANN and motion-capture retrieval literature.)

**Partitioning and the continuing-pose check**
- Unreal: Chooser tables return the databases to search from gameplay context (movement state, rotation mode). The Game Animation Sample searches walk databases only while walking, which "narrows the data Motion Matching searches" and improves performance. — [UE Game Animation Sample](https://dev.epicgames.com/documentation/unreal-engine/game-animation-sample-project-in-unreal-engine)
- Unreal: `MinCostAddend` stores the lowest cost addend across a database's poses, and searches are skipped when the running best cost is already below it (a cross-database lower bound). — [FSearchIndexBase](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/FSearchIndexBase)
- Unreal: `Continuing Pose Cost Bias` biases the current pose's cost (negative keeps it longer); `Pose Jump Threshold Time` and `Pose Reselect History` stop reselecting nearby or recent poses. — [Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)
- MxM tags can be enabled, disabled, or required at runtime to limit which clips match (community docs). — [Motion Matching for Unity user manual](https://motion-matching-for-unity.readthedocs.io/en/latest/user-manual/)
- Kinematica defines metrics, and therefore codebooks, per tag. — [Kinematica CodeBook API](https://docs.unity.cn/Packages/com.unity.kinematica@0.8/api/Unity.Kinematica.Binary.CodeBook.html)
- Holden seeds the best cost with the current frame's cost before scanning, so a good continuing pose prunes most boxes immediately. — [database.h](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/database.h)
- O3DE scores every candidate including the current frame, then switches only if the winner differs and is not within 0.1 s of the current frame in the same motion. — [O3DE MotionMatchingInstance.cpp](https://raw.githubusercontent.com/o3de/o3de/development/Gems/MotionMatching/Code/Source/MotionMatchingInstance.cpp)

### Inferences
- Bounding-box lower bounds stay valid under any non-negative per-dimension weights if the boxes are stored in unweighted normalized space and the query applies the weights per dimension when computing the box distance: each weighted per-axis box term is still ≤ the weighted per-axis point term. Runtime weight changes would then cost pruning efficiency, not correctness. Holden's code bakes weights into `features_scale`, so his stored bounds are tied to one weight set as written.
- The same argument holds for an axis-aligned KD-tree with backtracking that accumulates per-axis lower bounds. It does not hold for PCA (the principal axes depend on the weighted covariance), for VP-trees (partitions depend on the metric used at build), or for HNSW graphs (neighbor links depend on the build metric), so those need a rebuild when weights or channels change.
- Holden's boxes cover consecutive frames. Because animation is temporally smooth, 16-frame windows have tight per-dimension ranges, which is why a simple fixed-size grid prunes well without a tree build. Bound storage costs 2/16 + 2/64 ≈ 15.6% extra over the feature matrix.
- A full-cost re-rank of K candidates after an approximate broad phase (Unreal, O3DE) gives a quality versus cost knob (K) but loses the exactness guarantee of branch-and-bound.
- Partitioning by tag or database before searching reduces N directly and composes with any per-partition index.

### Gaps
- No published head-to-head benchmark of KD-tree versus AABB versus VP-tree versus HNSW on motion matching data was found.
- No source documents LSH or product quantization (in the FAISS sense) for motion matching at runtime.

## 3. Unreal Engine Pose Search plugin internals

### Takeaway
Unreal's Pose Search stores per-pose float feature vectors (`Values`) and compact per-pose metadata, normalizes channels across databases, and offers BruteForce, PCAKDTree (approximate KD-tree in PCA space plus a full-cost re-rank of K neighbors), and experimental VPTree and EventOnly modes. Memory savers include pose pruning of near-duplicates, PCA-value pruning, and stripping full values when K = 1. Runtime cost levers are search throttling, staggering, choosers, database LOD, and cached channel data. Epic publishes no timings or budgets.

### Cited Findings
**Search modes (UE 5.8 API docs)**
- `BruteForce` evaluates all indexed poses; `PCAKDTree` projects into PCA space with the most significant components and builds a KD-tree; `VPTree` and `EventOnly` (search only on events) are experimental and not for production. — [EPoseSearchMode](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/EPoseSearchMode)
- UE 5.3 also listed debug modes that compare BruteForce against PCAKDTree and validate PCAKDTree searches. — [UE 5.3 Python API: PoseSearchMode](https://docs.unrealengine.com/5.3/en-US/PythonAPI/class/PoseSearchMode.html)

**Database settings**
- `NumberOfPrincipalComponents`: "More dimensions allows a better explanation of the variance of the dataset that usually translates in better search results, but will imply more memory usage and worse performances." — [UE 5.3 Python API: PoseSearchDatabase](https://docs.unrealengine.com/5.3/en-US/PythonAPI/class/PoseSearchDatabase.html) (via search snippet)
- `KDTreeQueryNumNeighbors`: KD-tree results have approximate cost, so this many best poses get the full cost analysis; "If KDTreeQueryNumNeighbors is 1 all the SearchIndexPrivate::Values will be stripped away." `KDTreeQueryNumNeighborsWithDuplicates` caps full evaluations when PCA-value pruning merges duplicates (0 = no cap). Epic notes the property will be renamed to a KNN name so it also covers the VP-tree. — [UE 5.3 Python API: PoseSearchDatabase](https://docs.unrealengine.com/5.3/en-US/PythonAPI/class/PoseSearchDatabase.html) (via search snippet)
- Current docs list `KDTree Max Leaf Size`, `Pose Pruning Similarity Threshold` (merges similar poses under one index to save memory), `KNNQueryNumNeighbors`, `Continuing Pose Cost Bias`, `Looping Cost Bias`, `Exclude from Database Parameters` (trims clip start and end), and `Normalization Set` (normalizes across databases; "can yield more accurate and efficient results"). — [Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)

**Schema settings**
- Total cost is the sum of channel costs; each channel and each trajectory or pose sample adds memory and performance cost, and Epic recommends "as few channels as necessary." `Sample Rate` raises memory cost. `Number of Permutations` indexes the database multiple times. `Add Data Padding` pads data to 16-byte alignment for performance at extra memory. `Data Preprocessor`: None, Normalize, or Normalize Only by Deviation; weights are normalized automatically unless None. — [Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine)

**Index data structures (UE 5.8 API)**
- `FSearchIndexBase`: `Values` (`TAlignedArray<float>`), `ValuesVectorToPoseIndexes` (`FSparsePoseMultiMap<int32>`, the many-poses-to-one-vector map that pruning creates), `PoseMetadata` (`TAlignedArray<FPoseMetadata>`), `Assets`, `MinCostAddend`, `Stats`, and `PruneDuplicateValues(SimilarityThreshold, ...)`. — [FSearchIndexBase](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/FSearchIndexBase)
- `FSearchIndex`: `PCAValues` (values encoded in PCA space), `PCAProjectionMatrix`, `Mean`, `KDTree` (`FKDTree`), `VPTree` (`FVPTree`), `PCAValuesVectorToPoseIndexes`, and `WeightsSqrt` (square roots of the weights, stored "to reduce numerical error when comparing feature vectors, since the weights are multiplied by dataset variance"). Methods include `PCAProject`, `GetReconstructedPoseValues`, `PruneDuplicatePCAValues`, and `PrunePCAValuesFromBlockTransitionPoses`. — [FSearchIndex](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/FSearchIndex)
- `FPoseMetadata` per pose: `ValueOffset` (u32 offset into the values), `AssetIndex` (u32), `bBlockTransition` (bool), and `CostAddend` stored as `FFloat16`. — [FPoseMetadata](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/FPoseMetadata)
- `FKDTree(Count, Dim, const float* Data, MaxLeafSize, bUseRecursiveDivideTree)`, with KNN, filtered KNN, and radius result sets (max-heap variants experimental), and `GetAllocatedSize()`. — [FKDTree](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/FKDTree)
- Editor statistics report: `SchemaCardinality` (floats per pose), searchable frames, pruned frames, pruned PCA frames, and memory for `ValuesSize`, `PCAValuesSize`, `KDTreeSize`, `VPTreeSize`, `PoseMetadataSize`, and assets, plus PCA `ExplainedVariance`. No search-time statistic is listed. — [UPoseSearchDatabaseStatistics](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearchEditor/UPoseSearchDatabaseStatistics)

**Runtime cost levers on the Motion Matching node**
- `Search Throttle Time` (minimum time between searches); `Should Search` false makes the node search only when the current pose becomes invalid, which the docs say is useful "to stagger searches of different nodes for performance reasons"; `Should Use Cached Channel Data` lets databases with different schemas share query feature data (off by default). — [Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine); [motion matching debugging](https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-debugging-in-unreal-engine)
- The Game Animation Sample exposes a Motion Matching Database LOD setting (level of detail of the Pose Search Database assets) that can be scaled at runtime for performance, and uses the `CHT_PoseSearchDatabases` chooser to select databases. — [UE Game Animation Sample](https://dev.epicgames.com/documentation/unreal-engine/game-animation-sample-project-in-unreal-engine)

### Inferences
- `Values` is a flat aligned float array addressed by `ValueOffset`, with metadata in a separate array: a row-major (array-of-structures by pose) layout with an indirection that lets pruned duplicate poses share one vector.
- `WeightsSqrt` suggests weights fold into the compared vectors so a plain squared Euclidean distance applies; if so, channel weights are fixed at cook time for the PCA projection and KD-tree.
- With `KDTreeQueryNumNeighbors = 1`, only PCA values remain, so the database becomes a lossy PCA-compressed index and the result is approximate.

### Gaps
- Epic publishes no ms timings, per-character budgets, typical principal-component counts, or default K values; the API pages leave most members undocumented. The brute-force inner loop (SIMD or not), how the continuing pose is evaluated, VP-tree construction, PCA computation, and threading would need the engine source.
- No Unreal Fest talk with Pose Search performance figures was found.

## 4. O3DE Motion Matching gem (EMotionFX)

### Takeaway
O3DE stores features in a row-major N x M float matrix and uses a simple, approximate KD-tree broad phase: fixed-order axis splits at the median, depth capped at min(dimensions, 20), leaves of at least 1,000 frames, at most 48 dimensions, and a query that returns one leaf bucket without backtracking. A brute-force narrow phase scores those frames with per-feature cost factors. Search runs on a timer a few times per second.

### Cited Findings
- Feature matrix: N x M, one row per frame and one column per feature dimension, with a feature's columns forming a "feature block," stored "in row-major order for cache efficiency." Example: 108,000 frames x 59 columns ≈ 6.4 million values ≈ 24.3 MB. — [O3DE blog](https://docs.o3de.org/blog/posts/blog-motionmatching/)
- `FeatureMatrix` derives from a 2D matrix type and can report its memory usage; `FeatureMatrixTransformer` applies `MinMaxScaler` or `StandardScaler` to the matrix and the query vector. — [O3DE FeatureMatrix API](https://docs.o3de.org/docs/api/gems/motionmatching/class_e_motion_f_x_1_1_motion_matching_1_1_feature_matrix); [O3DE MotionMatching API index](https://docs.o3de.org/docs/api/gems/motionmatching/annotated)
- KD-tree build defaults: `m_maxKdTreeDepth = 20`, `m_minFramesPerKdTreeNode = 1000`. — [O3DE MotionMatchingData::InitSettings](https://docs.o3de.org/docs/api/gems/motionmatching/struct_e_motion_f_x_1_1_motion_matching_1_1_motion_matching_data_1_1_init_settings)
- KD-tree build: split dimension chosen by depth (dimension d at depth d), so each path splits each dimension at most once; median found with `nth_element` (O(n)); depth capped by `min(numDimensions, maxDepth)`; splitting stops below `2 x minFramesPerLeaf`; small leaves are deleted afterwards. Tree dimensions are a subset of the schema (`Feature::SetIncludeInKdTree(false)`), and the total must be 1 to 48 because "Going above a 48 dimensional tree would start eating up too much memory." Nodes are individually heap-allocated with parent and child pointers and a `size_t` vector of frame indices. — [O3DE KdTree.cpp](https://raw.githubusercontent.com/o3de/o3de/development/Gems/MotionMatching/Code/Source/KdTree.cpp)
- KD-tree query: descends by comparing against each node's median to a leaf (or a node lacking the child on the query's side) and returns that node's frame list wholesale (`resultFrameIndices = node->m_frames;`): no backtracking and no ranking, so the true nearest neighbor can be missed. — [O3DE KdTree.cpp](https://raw.githubusercontent.com/o3de/o3de/development/Gems/MotionMatching/Code/Source/KdTree.cpp)
- Narrow phase: the KD-tree is gated by the `mm_useKdTree` CVar; otherwise every frame is scored. Each feature's cost is multiplied by its cost factor (trajectory past and future have separate factors). Frames within 1.0 s of motion end are skipped inside the hot loop (a TODO says this should move to extraction). Profile scopes: `MM::BuildQueryVector`, `MM::BroadPhaseKDTree`, `MM::EvaluateQueryPose`. — [O3DE MotionMatchingInstance.cpp](https://raw.githubusercontent.com/o3de/o3de/development/Gems/MotionMatching/Code/Source/MotionMatchingInstance.cpp)
- Search frequency: a timer with interval `1 / m_lowestCostSearchFrequency`, reset after every search; the blog's example is 5 times a second. — [O3DE MotionMatchingInstance.cpp](https://raw.githubusercontent.com/o3de/o3de/development/Gems/MotionMatching/Code/Source/MotionMatchingInstance.cpp); [O3DE blog](https://docs.o3de.org/blog/posts/blog-motionmatching/)
- More KD-tree candidates improve quality but slow the search; the candidate count is controlled by maximum depth and minimum frames per leaf. — [O3DE blog](https://docs.o3de.org/blog/posts/blog-motionmatching/)

### Inferences
- With depth ≤ 20 and leaves ≥ 1,000 frames, a 108,000-frame database yields at most about 108 leaves, so each search narrow-phases at least about 1,000 frames (about 1% of the database), traded against misses near split planes.
- Because the narrow phase applies cost factors at runtime, O3DE's weights are adjustable without rebuilding, but its broad phase ignores weights, so a weight change does not change which bucket is searched.

### Gaps
- No O3DE timing numbers, character counts, or memory beyond the 24.3 MB example were published.

## 5. Data layout, SIMD, quantization, cache behavior, threading, and GPU search

### Takeaway
Every public implementation stores features as contiguous row-major f32 (one pose's dimensions adjacent), which suits per-pose early-out; vectorization (Burst auto-vectorization or manual) gives large constant-factor wins. Quantization is documented for metadata (Unreal `CostAddend` in f16), network weights (LMM int16), and Kinematica's per-float codebooks, but not for f16 or i8 feature matrices in a shipped system. Threading is per-character jobs; the only documented production-style GPU search is Traverse's exhaustive compute search for 250 characters.

### Cited Findings
- Holden: flat row-major `array2d<float>` for features and bounds, `array1d<float>` offsets and scales. — [database.h](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/database.h)
- O3DE: row-major "for cache efficiency." — [O3DE blog](https://docs.o3de.org/blog/posts/blog-motionmatching/)
- Unreal: `TAlignedArray<float>` values with optional 16-byte padding per pose for performance; `FFloat16` cost addend. — [Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine); [FPoseMetadata](https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/PoseSearch/FPoseMetadata)
- Unity thesis: C# classes "might not be allocated contiguously in memory," drastically decreasing search performance through cache misses, so features are stored as value types; Burst "automatically applies vectorization to some parts of the code"; a manually vectorized linear search cut LAFAN1 from 0.84 to 0.48 ms. — [arXiv 2310.05215](https://arxiv.org/html/2310.05215)
- Environment-aware MM: contiguous feature storage and sequential access help cache use and SIMD; search runs on the CPU in a single thread. — [arXiv 2510.22632](https://arxiv.org/html/2510.22632)
- MxM: runtime search runs in Burst-compiled jobs ("PoseSearch Job" marker); community docs give 0.5 to 2 ms per frame for one character on desktop or console and per-character targets under 2 ms desktop, 1.5 ms console, 5 ms mobile. Unverified community figures. — [MxM user manual](https://motion-matching-for-unity.readthedocs.io/en/latest/user-manual/); [MxM FAQ](https://motion-matching-for-unity.readthedocs.io/en/latest/faq/)
- Kinematica: each fragment float is encoded as a `kNumCodeBits`-bit index to its closest centroid. — [Kinematica CodeBook API](https://docs.unity.cn/Packages/com.unity.kinematica@0.8/api/Unity.Kinematica.Binary.CodeBook.html)
- LMM weights quantized to 16-bit integers halve memory (17 MB to 8.5 MB). — [Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)
- Traverse GPU (25 September 2026, merged 2026-09-11): `mm_match` uses one 256-thread workgroup per character; threads stride over matchable frames keeping a running best, then reduce with wave intrinsics and group-shared memory. Query build, normalization, search, and decision are each one dispatch across all characters; the search is exhaustive with no acceleration structure; ties break by lowest index so the result equals a sequential scan. The winner is written into a playback buffer read later in the same command buffer (no CPU readback; an 8-byte-per-character readback with one frame of latency was the original plan). The post does not mention SoA or f16. — [Traverse](https://traverse.nl/blog/motion-matching-on-the-gpu)
- Kiddo (Rust KD-tree) ships hand-written SIMD distance paths per target (`DistanceMetricAvx2`, `DistanceMetricAvx512`, `DistanceMetricNeon`, `DistanceMetricWasmSimd`) and supports f16 and u8/u16/u32 coordinates. — [kiddo DistanceMetric](https://docs.rs/kiddo/latest/kiddo/traits/dist/trait.DistanceMetric.html); [kiddo docs](https://docs.rs/kiddo/latest/kiddo/)

### Inferences
- Row-major per-pose storage favors the partial-distance early-out (stop after a few dimensions) and per-pose SIMD over D = 27 (3.4 AVX2 lanes of 8, so padding to 32 floats would cost about 18.5%). A column-major (structure of arrays) layout favors SIMD across 8 or 16 poses at once but defeats per-pose early-out. A blocked layout (AoSoA) matching the 16-frame AABB blocks, with dimensions interleaved across the block's frames, would let a surviving box be scored with full-width SIMD and no gather. No source reports this layout for motion matching.
- Feature memory at 100k poses x 27 dims: f32 10.8 MB, f16 5.4 MB, i8 2.7 MB. Lower precision raises how much of the database stays cache-resident per searching character, at the cost of quantization error in the cost ranking.
- Searches for different characters are independent, so the natural parallel unit is one search per job; splitting one search across threads would weaken the shared best-cost bound unless the bound is shared atomically.

### Gaps
- No public measurement of f16 or i8 feature matrices, AoSoA layouts, or NEON-specific motion matching kernels was found.
- No documented multi-threaded single-search split was found.

## 6. Level of detail and crowds

### Takeaway
Crowd-scale motion matching relies on searching less often (every 5 to 10 frames, or a few times per second), staggering search phases across characters, restricting the database per context, and updating distant characters less often. Reported scales are For Honor's 100+ characters with aggressive LOD, a 300+ character estimate from the Unity thesis, and Traverse's 250-character GPU capture. No public source reports thousands of motion-matched characters on the CPU.

### Cited Findings
- For Honor: aggressive character LOD avoids updating distant characters every frame, letting the system run on over 100 on-screen characters. — [Game Anim](https://www.gameanim.com/2016/05/03/motion-matching-ubisofts-honor/)
- Unity thesis: search every few frames (for example 10); estimate of 300+ characters within 1 ms at 0.03 ms per search. — [arXiv 2310.05215](https://arxiv.org/html/2310.05215)
- Environment-aware MM: search every 10 frames at 60 Hz (about 167 ms) while playback advances every frame, which lets search load spread across frames; "the reported performance scales linearly with the number of agents"; variance grows with database size, so the authors suggest a budget-based search capped at a maximum frame time. — [arXiv 2510.22632](https://arxiv.org/html/2510.22632)
- O3DE searches a few times per second (example: 5 Hz). Ubisoft's LMM blog says motion matching searches "every few frames" and assumes about 5 transitions per second. — [O3DE blog](https://docs.o3de.org/blog/posts/blog-motionmatching/); [Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)
- Unreal: Search Throttle Time, staggering with `Should Search`, Database LOD scaled at runtime, and choosers. — [Motion Matching in Unreal Engine](https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine); [UE Game Animation Sample](https://dev.epicgames.com/documentation/unreal-engine/game-animation-sample-project-in-unreal-engine)
- MxM: search every frame, every 2 frames, or every 3 frames, plus Pose Quality levels (community docs). — [MxM user manual](https://motion-matching-for-unity.readthedocs.io/en/latest/user-manual/)
- Traverse GPU, 250-character debug capture on an Apple M4: each character searches at 10 Hz by default with staggered timer phases; `mm_match` 0.18 to 0.36 ms per command-buffer context; `pose_inertialize` 0.17 to 0.25 ms; matcher dispatch 0.29 ms; debug-build numbers, "not targets." Memory: 790.96 MB empty world, 882.99 MB with one character, 908.60 MB with four, 1.50 GB for the 250-character run with animated acceleration structures (whole-app memory, not database only). The post notes brute-force cost scales with database size times searching characters, and gameplay cannot step through the decision on the CPU. — [Traverse](https://traverse.nl/blog/motion-matching-on-the-gpu)
- MIG 2023 (Lipschitz latent space + sparse mixture of experts, a learned successor to LMM): outperforms other implementations "for large character counts, by 8.5x times in CPU execution cost and at 80% of the memory requirements of Learned Motion Matching," and notes LMM reduced memory but increased performance cost. — [ACM DOI 10.1145/3623264.3624442](https://dl.acm.org/doi/10.1145/3623264.3624442) (abstract via [Unpaywall](https://unpaywall.org/10.1145%2F3623264.3624442))
- The Witcher 4 Unreal Engine 5.6 tech demo (State of Unreal 2025, base PS5, 60 fps) showed more than 300 animated characters on screen and highlighted Multi-Character Motion Matching (an interaction feature). The sources do not say the crowd itself is motion-matched. — [HotHardware](https://hothardware.com/news/witcher-4-unreal-engine-5-6-demo); [Inverse](https://www.inverse.com/gaming/the-witcher-4-new-setting-unreal-engine-tech-demo)

### Inferences
- At a 10-frame search interval with uniform staggering, only 1/10 of characters search each frame. At 0.07 to 0.22 ms per AABB-pruned search, that is about 45 to 140 searches per 10 ms of one core, so about 450 to 1,400 characters per core-frame budget of 10 ms, or proportionally fewer for a 1 ms animation budget (45 to 140 characters). Pruning efficiency varies with the query, so worst-case frames exceed these averages, which is the variance problem Environment-aware MM reports.
- "Cheaper schema for distant characters" is supported by Unreal's Database LOD and per-context databases; no source quantifies its saving.

### Gaps
- No shipped title publishes crowd-scale CPU motion matching counts beyond For Honor's "over 100."
- No source quantifies the cost saving of LOD schemas or database LOD.

## 7. Learned Motion Matching as a memory-compression technique

### Takeaway
LMM replaces the pose database with a decompressor (features + latent → pose, every frame), the feature database with a stepper (advances features + latent every frame between searches), and the search with a projector (query → features + latent, at transitions). On Ubisoft's large controller it cut memory from 590 MB to about 17 MB (8.5 MB with int16 weights, 70x), with roughly constant CPU cost per character that is higher than a pruned search on small databases.

### Cited Findings
- Memory: classic motion matching memory grows with the features database, which scales with feature count and data; large controller 590 MB versus about 17 MB of LMM weights, 8.5 MB with 16-bit quantization ("70x"); about 10x saving on the first path-following example. LMM CPU cost is "constant" as data grows. — [Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)
- Final model needs no stored animation data or matching metadata; memory "typically" stays small as data grows. — [theorangeduck: Learned Motion Matching](https://www.theorangeduck.com/page/learned-motion-matching)
- Runtime roles: Decompressor every frame (features + extra latent → full pose); Stepper every frame between transitions (predicts next features + latent; small because it predicts over short spans); Projector at transitions (query → features + latent, replacing search). The decompressor alone shows occasional visible errors (for example hand positions), fixed mostly by the extra auto-encoded features. — [Ubisoft La Forge](https://www.ubisoft.com/en-us/studio/laforge/news/6xXL85Q3bF2vEj76xmnmIu/introducing-learned-motion-matching)
- Holden's open-source networks: compressor 4 linear layers (512 hidden, ELU), latent size 32; decompressor 2 layers (input features + 32 → 512 ReLU → `15*(nbones-1) + 6 + nextra`). — [train_decompressor.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/train_decompressor.py)
- Stepper: 3 layers (features + latent → 512 → 512 → features + latent, ReLU), outputs velocities integrated at dt = 1/60, trained over 20-frame windows. — [train_stepper.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/train_stepper.py)
- Projector: 5 layers (features → 4 x 512 ReLU → features + latent), trained on noisy queries with the nearest database frame (BallTree) as target. — [train_projector.py](https://raw.githubusercontent.com/orangeduck/Motion-Matching/main/resources/train_projector.py)
- Changing weights or matching properties requires regenerating the database and retraining the networks. — [orangeduck/Motion-Matching README](https://github.com/orangeduck/Motion-Matching)
- A hobbyist reimplementation (JavaScript or TypeScript stack, two minutes of mocap) measured 0.73 MB learned versus 7.1 MB database, but 0.26 ms versus 0.027 ms per step, noting "On two minutes of mocap, the exact search is still faster." Single unreviewed test. — [monjar study PR](https://github.com/monjar/Advanced-Procedural-Animations-Study/pull/5)
- The MIG 2023 successor reports LMM "resulted in increases in performance costs" and claims 8.5x lower CPU cost at 80% of LMM memory for large character counts. — [ACM DOI 10.1145/3623264.3624442](https://dl.acm.org/doi/10.1145/3623264.3624442)

### Inferences
- Parameter counts from the repo architectures with D = 27 and latent 32: projector ≈ 27x512 + 3x512² + 512x59 ≈ 0.83 M parameters (≈ 3.3 MB f32), run only at searches; stepper ≈ 59x512 + 512² + 512x59 ≈ 0.32 M (≈ 1.3 MB), every frame; decompressor ≈ 59x512 + 512 x out, with out ≈ 338 for an assumed 23-bone skeleton with 2 contact values ≈ 0.20 M (≈ 0.8 MB), every frame. Per character per frame that is about 0.5 M multiply-adds steady-state plus about 0.8 M per search, independent of database size. Many characters can be batched into matrix-matrix products.
- The break-even point is database size: for small databases a pruned exact search (0.03 to 0.09 ms) is cheaper than per-frame network inference; LMM pays off when memory, not CPU, is the constraint.

### Gaps
- The paper's own Table 3 timings and memory breakdown could not be read (the PDF exceeded the fetch limit, and the ACM pages returned 403).

## 8. Rust crates for nearest-neighbor search

### Takeaway
No Rust motion matching crate exists. Kiddo is the best-maintained KD-tree, with SIMD and f16, but it targets under about 10 dimensions and its metrics are stateless associated functions, so runtime weights must be baked into coordinates. `kdtree` takes a distance function per query. HNSW crates (`hnsw_rs`, `usearch`) support stateful custom metrics and predicate filtering but are approximate and built for one metric. `vpsearch` needs a true (non-squared) metric. For a 27-dimension weighted query, the crates offer building blocks rather than a direct fit.

### Cited Findings
| Crate | Latest (date) | Downloads total / recent | Relevant facts |
|---|---|---|---|
| `kiddo` | 6.3.0 (2026-09-06) | 8.13 M / 0.89 M | `ImmutableKdTree` and `MutableKdTree`; f64, f32, f16 (`half`), fixed-point, u8/u16/u32; `nearest_one`, `nearest_n`, `within`, `best_n`, `approx`, batch queries, dual-tree joins; `simd` feature needs nightly; `rkyv_08` zero-copy; rayon parallel APIs by default; designed for low dimensions (under about 10D). — [kiddo docs](https://docs.rs/kiddo/latest/kiddo/); [crates.io](https://crates.io/crates/kiddo) |
| `kdtree` | 0.8.1 (2026-07-23) | 3.25 M / 0.79 M | `nearest(&point, n, &squared_euclidean)` takes a distance function argument. — [kdtree-rs](https://github.com/mrhooray/kdtree-rs); [crates.io](https://crates.io/crates/kdtree) |
| `kd-tree` | 0.6.2 (2025-11-16) | 0.82 M / 0.15 M | General k-dimensional tree. — [crates.io](https://crates.io/crates/kd-tree) |
| `hnsw_rs` | 0.3.5 (2026-10-09) | 1.23 M / 0.52 M | HNSW (Malkov and Yashunin); distances from `anndists`; filter trait for searches; `hnswio` dump and reload; `datamap` memory-maps vectors. — [hnsw_rs docs](https://docs.rs/hnsw_rs/latest/hnsw_rs/); [crates.io](https://crates.io/crates/hnsw_rs) |
| `usearch` | 2.26.5 (2026-10-18) | 1.57 M / 0.72 M | Approximate (HNSW); f32, f64, f16, i8, u8, b1x8; SIMD distances; `MetricFunction` and `StatefulMetric` custom callbacks; `StatefulPredicate` filtering. — [usearch docs](https://docs.rs/usearch/latest/usearch/); [crates.io](https://crates.io/crates/usearch) |
| `instant-distance` | 0.6.1 (2023-06-26) | 1.15 M / 0.45 M | HNSW; custom distance via its `Point` trait; no release since 2023. — [instant-distance](https://github.com/djc/instant-distance); [crates.io](https://crates.io/crates/instant-distance) |
| `vpsearch` | 2.1.0 (2025-10-08) | 73 k / 2.4 k | `MetricSpace::distance(&self, other, &UserData)`; requires a true metric, not squared; `find_nearest`. — [vpsearch docs](https://docs.rs/vpsearch/latest/vpsearch/) |
| `petal-neighbors` | 0.19.0 (2026-09-29) | 306 k / 47 k | Ball tree and VP tree over `ndarray`. — [petal-neighbors](https://github.com/petabi/petal-neighbors); [crates.io](https://crates.io/crates/petal-neighbors) |
| `acap` | 0.4.0 (2025-02-18) | 55 k / 8.8 k | "As Close As Possible" nearest-neighbor search. — [crates.io](https://crates.io/crates/acap) |
| `faiss` | 0.13.0 (2025-11-15) | 146 k / 28 k | Bindings to the FAISS C++ library. — [crates.io](https://crates.io/crates/faiss) |
| `simsimd` | 6.5.16 (2026-03-07) | 3.71 M / 1.83 M | Mixed-precision SIMD vector math for x86 and ARM (distance kernels, not an index). — [crates.io](https://crates.io/crates/simsimd) |
| `space` | 0.19.0 (2025-08-04) | 1.42 M / 0.30 M | rust-cv abstractions for spatial data structures and search. — [crates.io](https://crates.io/crates/space) |

- Kiddo's metric traits use associated functions without `self`: `fn dist1(a: Self::Output, b: Self::Output) -> Self::Output` (single-axis contribution), `fn dist<const K: usize>(a: &[Self::Output; K], b: &[Self::Output; K])`, and `combine_component` (merges per-axis contributions into point or box distances). Implementors: `SquaredEuclidean`, `Manhattan`, `Chebyshev`, `Minkowski`. The trait is not dyn compatible. — [kiddo DistanceMetricScalar](https://docs.rs/kiddo/latest/kiddo/traits/dist/trait.DistanceMetricScalar.html); [kiddo DistanceMetric](https://docs.rs/kiddo/latest/kiddo/traits/dist/trait.DistanceMetric.html)
- `anndists::Distance::eval(&self, va: &[T], vb: &[T]) -> f32` takes `&self`, so an implementor can carry state such as weights; implementors include `DistL1`, `DistL2`, `DistCosine`, `DistFn`, and `DistPtr`. — [anndists Distance](https://docs.rs/anndists/latest/anndists/dist/distances/trait.Distance.html)
- A search for a Bevy or Rust motion matching crate found none; `bevy_motion` is a physics engine, and `bevy_spatial` is an entity spatial index. — search results: [docs.rs bevy_motion](https://docs.rs/bevy_motion)

### Inferences
- Kiddo's stateless metrics mean per-query weights cannot be passed in; options are scaling coordinates by sqrt(weight) at build time (rebuild on weight change) or a type-level constant metric. Its per-axis `dist1` plus `combine_component` structure is the per-axis lower-bound accumulation that branch-and-bound needs, and its low-dimension design matches PCA-reduced features (Unreal's approach) better than raw 27D vectors.
- HNSW and VP-tree crates bind the metric at build time; a stateful metric lets weights be read at query time, but the graph or partition was built under the old weights, so results degrade silently rather than fail.
- Holden's AABB scheme needs no crate: two min and max tables and a loop, which also avoids KD-tree dimensionality limits.

### Gaps
- Kiddo's maximum practical dimension and performance at 27D were not benchmarked in any source found.
- The `kdtree` crate's distance-function signature (whether it accepts closures capturing weights) was not confirmed from documentation.
- Maintenance status is inferred from release dates and download counts only; issue activity was not checked.
