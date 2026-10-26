# Attribution and thanks

Thank you to the authors who shared the following code, research, and design
ideas. Each credit names the contribution used in the engine.

| People | Published work | Contribution used in Byte Engine |
| --- | --- | --- |
| Ryan Fleury | [Multi-Core By Default](https://www.dgtlgrove.com/p/multi-core-by-default) | Alley's execution model: lanes run the same function, narrow to serial work, and share results before continuing in parallel. |
| Eric Lengyel | [Slug reference shaders](https://github.com/EricLengyel/Slug) | Curve and band data layout and the adapted coverage shader for UI text and filled paths. |
| Troy Sobotka and Benjamin Wrensch | [Minimal AgX Implementation](https://iolite-engine.com/blog_posts/minimal_agx_implementation) | Sobotka's AgX display transform and Wrensch's polynomial approximation used by the AgX tone mapper. |
| John Hable, James McLaren, and Stephen Hill | [Visibility Buffer Rendering with Material Graphs](https://filmicworlds.com/blog/visibility-buffer-rendering-with-material-graphs/) | Barycentric coordinates and derivatives from the published calculation, including McLaren and Hill's corrected gradients. |
| Evan Wallace | [Fast Rounded Rectangle Shadows](https://madebyevan.com/shaders/fast-rounded-rectangle-shadows/) | UI shadow integration that combines an analytic row integral with four samples along the other axis. |
| Brian Karis and Dimitar Lazarov | [Physically Based Shading on Mobile](https://www.unrealengine.com/en-US/blog/physically-based-shading-on-mobile) | Karis's environment BRDF approximation, which builds on Lazarov's work. |
| Krzysztof Narkowicz | [ACES Filmic Tone Mapping Curve](https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/) | The five-coefficient filmic curve fit used by the ACES tone mapper. |
| Melissa O'Neill, Mark Jarzynski, and Marc Olano | [PCG](https://www.pcg-random.org/paper.html) and [Hash Functions for GPU Rendering](https://jcgt.org/published/0009/03/02/) | O'Neill's 32-bit permutation, used as a particle hash as studied by Jarzynski and Olano. |
| Tom Duff, James Burgess, Per Christensen, Christophe Hery, Andrew Kensler, Max Liani, and Ryusuke Villemin | [Building an Orthonormal Basis, Revisited](https://jcgt.org/published/0006/01/01/) | The stable tangent frame used by CPU and GPU environment baking and diffuse screen-space sampling. |
| Mikko Mononen | [Simple Stupid Funnel Algorithm](https://digestingduck.blogspot.com/2010/03/simple-stupid-funnel-algorithm.html) | The funnel restart rule used to straighten navigation paths through portals. |
| Jorge Jimenez, Jose I. Echevarria, Belen Masia, Fernando Navarro, and Diego Gutierrez | [SMAA](https://github.com/iryoku/smaa) | The adapted spatial antialiasing shaders and their area and search lookup textures. |
| Jorge Jimenez | [Next Generation Post Processing in Call of Duty: Advanced Warfare](https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare) | The 13-tap bloom downsampling filter and interleaved gradient noise. |
| Alan Wolfe | [Interleaved Gradient Noise: A Different Kind of Low Discrepancy Sequence](https://blog.demofox.org/2022/01/01/interleaved-gradient-noise-a-different-kind-of-low-discrepancy-sequence/) | The discussion of temporal noise offsets and mapping scalar noise to two dimensions used by screen-space sampling. |
| Sébastien Hillaire | [A Scalable and Production Ready Sky and Atmosphere Rendering Technique](https://sebh.github.io/publications/egsr2020.pdf) | The multiple-scattering approximation used by the atmosphere renderer. |
| Box2D authors | [Contact Recycling](https://box2d.org/posts/2026/10/contact-recycling/) | Reusing contact geometry while two bodies remain close to their previous relative placement. |
| Gino van den Bergen | [Ray Casting against General Convex Objects with Application to Continuous Collision Detection](https://www.dtecta.com/papers/jgt04raycast.pdf) | The support-mapped convex ray cast used for rounded shapes. |
| Inigo Quilez | [2D distance functions](https://iquilezles.org/articles/distfunctions2d/) | The distance-function formulations referenced by UI sector coverage and the closest-point calculation for quadratic curves. |

License notices for adapted code and assets are collected in
[Third-party notices](THIRD_PARTY_NOTICES.md).
