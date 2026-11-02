# Third-party notices

Byte-Engine's own code is covered by [LICENSE](LICENSE). The following notices
apply to the identified adaptations and assets. Source and design credits are
listed in [Attribution and thanks](docs/attribution.mdx).

## Slug coverage shader

The glyph and filled-path coverage code in
`crates/byte-engine/assets/rendering/ui/ui-fragment.besl` is adapted from
[Eric Lengyel's Slug reference shaders](https://github.com/EricLengyel/Slug).
The adaptation uses the MIT license option. Its notice is also retained in the
shader source.

MIT License

Copyright (c) 2017 Eric Lengyel

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Minimal AgX polynomial approximation

The polynomial approximation in
`crates/byte-engine/assets/rendering/agx/tone-mapping.besl` follows
[Benjamin Wrensch's Minimal AgX Implementation](https://iolite-engine.com/blog_posts/minimal_agx_implementation)
of Troy Sobotka's AgX display transform. This notice covers that approximation;
it is also retained in the shader source.

MIT License

Copyright (c) 2024 Missing Deadlines (Benjamin Wrensch)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Fitted ACES transforms

The `aces_fitted_*` functions in
`crates/byte-engine/assets/rendering/layouts/color.besl`, which the ACES tone
mapper and the ACES color-grading pass use, adapt Stephen Hill's fit of the ACES
reference rendering and sRGB output transforms from
[Baking Lab's ACES.hlsl](https://github.com/TheRealMJP/BakingLab/blob/master/BakingLab/ACES.hlsl).
This notice is also retained in the layout source.

MIT License

Copyright (c) 2016 MJP

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## ACES 2.0 output transform

`crates/byte-engine/assets/rendering/color-grading/aces2.besl`,
`scripts/generate-aces2-tables.py`, and the tables that script generates in
`crates/byte-engine/assets/rendering/layouts/aces2_rec709_100nit.besl` port the
forward ACES 2.0 output transform from the Academy's reference implementation in
[aces-core](https://github.com/aces-aswf/aces-core)
(`lib/Lib.Academy.OutputTransform.ctl` and `lib/Lib.Academy.Tonescale.ctl`),
as its Rec.709-D65 100-nit sRGB-piecewise output runs it. The port changes the
language and leaves out the inverse transform and the other displays.

Copyright Contributors to the ACES Project.

Licensed under the Apache License, Version 2.0 (the "License"); you may not use
these files except in compliance with the License. You may obtain a copy of the
License at http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed
under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR
CONDITIONS OF ANY KIND, either express or implied. See the License for the
specific language governing permissions and limitations under the License.

## Hable filmic tone curve

The tone curve in `crates/byte-engine/assets/rendering/hable/tone-mapping.besl`,
which the Hable tone mapper uses, is an independent implementation of the
formula and constants John Hable published in
[Filmic Tonemapping Operators](http://filmicworlds.com/blog/filmic-tonemapping-operators/)
and presented in the GDC 2010 talk
[Uncharted 2: HDR Lighting](https://gdcvault.com/play/1012459/Uncharted-2-HDR).
It is written from the published formula, not adapted from source code. This
credit is also retained in the shader source.

## SMAA shaders and lookup textures

The shaders and lookup textures in
`crates/byte-engine/assets/rendering/smaa` are adapted from
[the SMAA reference implementation](https://github.com/iryoku/smaa).
The existing `LICENSE.txt` in that directory contains the following notice.

Copyright (C) 2013 Jorge Jimenez (jorge@iryoku.com)
Copyright (C) 2013 Jose I. Echevarria (joseignacioechevarria@gmail.com)
Copyright (C) 2013 Belen Masia (bmasia@unizar.es)
Copyright (C) 2013 Fernando Navarro (fernandn@microsoft.com)
Copyright (C) 2013 Diego Gutierrez (diegog@unizar.es)

Permission is hereby granted, free of charge, to any person obtaining a copy
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software. As clarification, there is no
requirement that the copyright notice and permission be included in binary
distributions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Surfel global illumination

The surfel GI shaders, `crates/byte-engine/assets/rendering/visibility/surfel-*.besl`
other than the temporal and spatial passes, and their layers,
`crates/byte-engine/assets/rendering/visibility/layouts/surfel_gi.besl` and
`crates/byte-engine/assets/rendering/visibility/layouts/surfel_resolve.besl`,
adapt the surfel lifecycle, grid, resolve, ray tracing, ray guiding, radial
depth occlusion, and multiscale mean estimator of
[webgiya by Jure Triglav](https://github.com/jure/webgiya). The
copyright line is also retained in `layouts/surfel_gi.besl`.

MIT License

Copyright (c) 2026 Jure Triglav

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Surfel GI reconstruction and hit spawning

The surfel GI history blend and spatial pass,
`crates/byte-engine/assets/rendering/visibility/layouts/surfel_temporal.besl` and
`crates/byte-engine/assets/rendering/visibility/surfel-spatial.besl`, adapt the
temporal and spatial filters of the ray-traced diffuse GI in
[kajiya by Tomasz Stachowiak and Embark Studios](https://github.com/EmbarkStudios/kajiya):
a history that counts its samples up to a limit and gives most of them up when
the light changes past its variance, and an edge-aware spiral filter whose
radius shrinks as the history grows confident, which averages light through a
reversible tone map and feeds nothing back. The surfels that the shade pass,
`crates/byte-engine/assets/rendering/visibility/surfel-shade.besl`, and the hit
allocation, `crates/byte-engine/assets/rendering/visibility/surfel-allocate-hits.besl`,
spawn where rays hit surfaces no surfel covers follow kajiya's irradiance cache,
which allocates entries where rays look them up, limited by the lookup's
bounce. Embark Studios dual-licenses kajiya under the MIT license and the
Apache License, Version 2.0; the adaptation uses the MIT license option. The copyright line is
also retained in these shaders.

MIT License

Copyright (c) 2019 Embark Studios

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## cinegrain2 film grain model

The film grain model in `crates/byte-engine/assets/rendering/film_grain/` and
`crates/byte-engine/src/rendering/render_passes/film_grain.rs` is adapted from
[cinegrain2](https://github.com/mr-berndt/cinegrain2) by mr-berndt, as published
under the MIT license up to commit `fd9e8f3` (12 October 2026). Later versions
of cinegrain2 are licensed under the GPL and were not used. The adaptation
keeps the grain shape, the luma equalizer, the variance normalization, and the
downscale retention, and replaces the noise hash, the pass layout, and the
parameter plumbing.

MIT License

Copyright (c) 2026 mr-berndt

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
