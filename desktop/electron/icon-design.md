# Columbina application icon

The application icon is a small silver-white dove on a deep blue-gray rounded tile.
It was generated with the built-in `image_gen` tool, with `transparent_background: true`.
No character illustration was used as a source or reference. The outside corners
are transparent.

The source artwork lives in `desktop/assets/columbina-icon.png`, as described in
`../ASSETS.md`. Build preparation copies it to the ignored `desktop/public/columbina-icon.png`.
This AI-generated artwork and the derived icons are outside the source code's MIT license.
`desktop/electron/create-icon.ps1` fits the entire image into each square, preserves
its alpha, and writes the six PNG-backed Windows icon sizes: 16, 32, 48, 64, 128,
and 256 pixels. It does not crop the dove or rounded tile. On macOS,
`desktop/electron/create-icon.sh` resizes the square artwork with `sips` into the ten
images of an icon set, from 16 to 1024 pixels, and `iconutil` writes `icon.icns` from them.

## Generation prompt

```text
Use case: logo-brand.
Asset type: a production Windows desktop application icon, square, legible from 16px to 256px.
Primary request: one exceptionally restrained, elegant small dove symbol. A silver-white geometric silhouette with a clearly recognizable rounded dove head, a short small beak, a gently curved compact body, and one broad wing held slightly upward. Build the entire bird from just a few confident flowing shapes. Quiet, refined, balanced proportions and generous deliberate negative space. The result should feel like a carefully drawn professional software identity, not a cartoon illustration.
Composition: center the bird on a deep blue-gray rounded-square tile. The tile occupies about 90% of the square canvas, with truly transparent space outside its rounded corners. The bird occupies about 65% of the tile and remains unmistakable at tiny taskbar sizes.
Palette: flat deep blue-gray tile around #151C2A and a flat warm silver-white bird around #EBEDF6; at most an extremely subtle cool lavender undertone, no additional colors.
Style: crisp clean flat vector-like edges, bold simple masses, high contrast, sophisticated minimalism.
Constraints: exactly one dove and one rounded-square background tile, genuine alpha transparency outside the tile; no text, letters, numbers, border ornament, moon, stars, branches, halo, feather lines, decorative particles, intricate detail, realistic feathers, gradients, gloss, 3D extrusion, lighting effects, shadows, mockup scene, or extra icons. Do not crop the tile.
```

