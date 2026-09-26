# SemanticDB brand assets

The SVG set lives in `docs/assets`. The mark is a hand-reconstructed vector version of the approved curved semantic triangle, preserving the agent, human, and database silhouette. It uses smooth cubic curves with transparent internal openings. The raster texture has been removed; no teal or antenna is present.

![Asset preview](semanticdb-assets-preview.png)

## Files

Each of these four families has four variants, for 16 transparent SVGs:

| Family | Default file | Use |
| --- | --- | --- |
| Mark | `semanticdb-mark.svg` | Standalone emblem |
| Horizontal | `semanticdb-horizontal.svg` | Website header, README, horizontal branding |
| Stacked | `semanticdb-stacked.svg` | Centered branding, title cards |
| Wordmark | `semanticdb-wordmark.svg` | SemanticDB lettering alone |

| Filename suffix | Foreground | Intended background |
| --- | --- | --- |
| No suffix | Charcoal `#252e33` | Light |
| `-white.svg` | White `#ffffff` | Dark |
| `-black.svg` | Black `#000000` | Light, monochrome printing |
| `-currentcolor.svg` | CSS `currentColor` | Set by the surrounding interface when inlined |

Three additional assets bring the set to 19 SVGs:

- `semanticdb-app-icon-light.svg`: charcoal mark on an opaque ivory `#fbf8f0` square, with extra safe padding.
- `semanticdb-app-icon-dark.svg`: ivory mark on an opaque charcoal square, with extra safe padding.
- `favicon.svg`: rounded background tile with automatic light/dark colors through `prefers-color-scheme`. Defaults to light when the renderer does not support the media query.

The app icons are SVG source artwork. Platform-specific PNG or ICO packaging can be derived from them when needed.

## Typography and rendering

The wordmark is **Avenir Next Demi Bold**, converted to vector outlines using CoreText. All distributed logo SVGs contain paths rather than live text, embedded fonts, or raster images. They render without an installed font, network access, or external resources.

Use the default horizontal file for a light website header and its `-white` sibling for a dark header. Preserve the viewBox aspect ratio; do not stretch the logo. The mark has built-in padding. Allow additional surrounding space where practical.

For the full three-part detail, display the standalone mark at 32 CSS pixels or larger. At 16 pixels the favicon keeps the silhouette, but the agent eyes and database detail become subtle. The preview includes checks at 16, 24, 32, 48, and 64 pixels.

The `currentColor` variants inherit a surrounding CSS color only when the SVG markup is inlined. An SVG loaded with an HTML `img` tag does not inherit the parent element's text color; use a fixed-color file in that case. When using an `img`, supply an appropriate `alt`, or an empty `alt` when an adjacent visible name already provides the same accessible label.

## Source and regeneration

`docs/assets/source/approved-concept.png` preserves the selected concept. `build_assets.py` contains the editable Bezier geometry, placement, palette, and SVG generation. `wordmark-path.json` holds the outlined lettering, font name, and dimensions.

Regenerate the 19 SVGs from the repository root using Python 3 with no third-party packages:

```sh
python3 docs/assets/source/build_assets.py
```

`outline-wordmark.swift` preserves the original font-to-path conversion. It is only needed if changing the wordmark; running the asset generator does not require macOS or font installation. On macOS with Avenir Next installed, regenerate its data with:

```sh
swift docs/assets/source/outline-wordmark.swift docs/assets/source/wordmark-path.json
```

The overview is in `docs/generated/semanticdb-assets-preview.svg`, with a PNG companion for convenient viewing. Its typography labels are live text; the 19 deliverable SVG assets all use outlined artwork.

## Validation

All 19 assets were parsed as XML, checked for external references and embedded raster images, and rendered successfully. The primary and reversed marks, horizontal and stacked compositions, and small-size favicon examples were visually inspected. The vector is a smooth reconstruction of the approved raster rather than an exact pixel trace.
