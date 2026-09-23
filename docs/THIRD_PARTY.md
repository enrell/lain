# Third-party notices

## Anime4K

Lain's `Anime4K DoG x2` shader pack adapts the Anime4K algorithm and
coefficients published at <https://github.com/bloc97/Anime4K>. The WebGL2
fallback bundles the original MIT-licensed mpv GLSL hooks for Mode A, Mode
A+A and Lite in `web/src/lib/player/webgl/packs/`.

```text
MIT License

Copyright (c) 2019 bloc97

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
```

## anime4k-wgpu

The pack's WGSL implementation is adapted from the four `upscale_dog_x2`
passes published at <https://github.com/SegaraRai/anime4k-wgpu>. Lain changes
the compute/storage-texture interface into its fragment-pass ABI while
retaining the algorithm and coefficients.

The Mode A, Mode A+A and Lite compute presets include selected generated WGSL
and pipeline metadata from `anime4k-wgpu` commit `526f5808` in
`web/src/lib/player/webgpu/packs/anime4k-predefined.json`. They retain the
upstream MIT terms below. The original GLSL chains on this machine were used
to select and order the corresponding modules; no mpv configuration file is
bundled.

```text
MIT License

Copyright (c) 2026 SegaraRai

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
```
