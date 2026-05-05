# VRChat Chatbox Reference

This document is the canonical implementation reference for the fixed VRChat chatbox wrap model used by `VRC Live Caption`. It keeps the project-facing layout, font, validation, and line-break facts needed to simulate VRChat wrapping and clipping without the reverse-engineering narrative.

## Overview

- The chatbox model is a fixed TMP text layout, not a configurable heuristic width model.
- Long-text behavior is defined by a fixed text rectangle, fixed margins, fixed font size, the VRChat font/fallback stack, real glyph widths, TMP line-break tables, and a hard `9`-line cap.
- For this project, the important target is text layout and clipping inside the `ChatText` rectangle, not full world-space chat bubble rendering.

## Verified layout contract

### Text object paths

- Chat content:
  - `VRCPlayer > NameplateContainer > ChatBubble > Canvas > Chat > ChatText`
  - `VRCPlayer > NameplateContainer > ChatBubbleMirror > Canvas > Chat > ChatText`
- Typing indicator:
  - `VRCPlayer > NameplateContainer > ChatBubble > Canvas > TypingIndicator > Text`
  - `VRCPlayer > NameplateContainer > ChatBubbleMirror > Canvas > TypingIndicator > Text`

### ChatText TMP fields

These values are high-confidence extracted values for both normal and mirrored `ChatText`.

| Field | Value |
|---|---:|
| `fontSize` | `18.0` |
| `fontSizeBase` | `18.0` |
| `fontWeight` | `400` (`Regular`) |
| `fontStyle` | `Normal` |
| `enableAutoSizing` | `false` |
| `fontSizeMin` | `16.0` |
| `fontSizeMax` | `26.0` |
| `characterSpacing` | `0.0` |
| `wordSpacing` | `0.0` |
| `lineSpacing` | `0.0` |
| `lineSpacingAdjustment` | `0.0` |
| `paragraphSpacing` | `0.0` |
| `characterWidthAdjustment` | `0.0` |
| `textWrappingMode` | `Normal` |
| `wordWrappingRatios` | `0.4` |
| `margin` | `(10, 10, 10, 10)` |
| horizontal alignment | `Center` |
| vertical alignment | `Middle` |

Notes:

- Treat wrapping as enabled in practice. `enableWordWrapping` was not independently confirmed as a separate field, but runtime behavior wraps text and `textWrappingMode = Normal` was extracted directly.
- Canvas/container scaling should not be included in the wrap-width formula. Wrapping is driven by the local `ChatText` rectangle plus TMP settings.

### ChatText RectTransform

These values apply to both normal and mirrored `ChatText`.

| Field | Value |
|---|---|
| `sizeDelta` | `(300, 265)` |
| `anchorMin` | `(0.5, 0.5)` |
| `anchorMax` | `(0.5, 0.5)` |
| `anchoredPosition` | `(0, 0)` |
| `pivot` | `(0.5, 0.5)` |

### Chat container and canvas

- `Chat` stretches to the parent `Canvas`.
- `Canvas` local scale is `(2, 2, 2)`.
- parent `ChatBubble` local scale is `(0.5, 0.5, 0.5)`.
- These scales should not be included in the wrap-width formula. Wrapping is driven by the local `ChatText` rectangle plus TMP settings.

### Derived layout limits

- fixed rect: `300 × 265`
- margin: `(10, 10, 10, 10)`
- usable size: `280 × 245`
- max visible lines: `9`

### TypingIndicator parameters

TypingIndicator uses the same primary font asset but a different text configuration:

- `fontSize = 40`
- `margin = (0, 5, 0, 5)`
- `characterSpacing = 0`
- `wordSpacing = 0`
- `lineSpacing = 0`
- `enableAutoSizing = false`

## Fonts and fallbacks

### Primary fonts

- primary TMP font asset: `NotoSans-Regular SDF`
- primary raw font: `NotoSans-Regular`
- primary raw font PostScript name: `NotoSans-Regular`
- chatbox material: `NotoSans-Regular SDF Nameplates ChatBubble`
- SDF atlas: `NotoSans-Regular SDF Atlas`

`NotoSans-Regular` is the direct width model for Latin text. The raw font name table confirms:

- family: `Noto Sans`
- style: `Regular`
- full name: `Noto Sans Regular`
- PostScript: `NotoSans-Regular`
- version: `Version 2.000;GOOG;...`

### Fallbacks

- primary CJK fallback: `NotoSansCJK-JP-Regular SDF`
- primary CJK raw font: `NotoSansCJK-JP-Regular`
- emoji fallback is present: `NotoEmoji-Regular SDF`

For Chinese, Japanese, and full-width punctuation, use `NotoSansCJK-JP-Regular` as the primary width model before considering later fallbacks.

### Observed fallback chain

Observed local fallback order in `NotoSans-Regular SDF`:

1. `VRCCustom SDF`
2. `NotoEmoji-Regular SDF`
3. `NotoSansCJK-JP-Regular SDF`
4. `NotoSansHebrew-Regular SDF`
5. `NotoSansArabic-Regular SDF`
6. `NotoSansThai-Regular SDF`
7. `NotoSansArmenian-Regular SDF`
8. `NotoSansBengali-Regular SDF`
9. `NotoSansDevanagari-Medium SDF`
10. `NotoSansGeorgian-Regular SDF`
11. `NotoSansGujarati-Regular SDF`
12. `NotoSansGurmukhi-Regular SDF`
13. `NotoSansKannada-Regular SDF`
14. `NotoSansLao-Regular SDF`
15. `NotoSansMalayalam-Regular SDF`
16. `NotoSansOriya-Regular SDF`
17. `NotoSansTamil-Regular SDF`
18. `NotoSansTelugu-Regular SDF`
19. `NotoSansTibetanV-Regular SDF`

Notes:

- This fallback chain is a high-confidence inference from the raw `TMP_FontAsset` `PPtr` array, not a full typetree text export.
- `sharedassets0.assets` also contains `NotoSansCJK-SC/TC/KR` assets, but the directly observed local CJK primary fallback for this chatbox remains `NotoSansCJK-JP-Regular SDF`.

## Line-break rules

VRChat ships TMP line-break resources in `resources.assets`. These tables are more authoritative than handwritten punctuation rules.

### Leading characters

```text
([｛〔〈《「『【〘〖〝‘“｟«$—…‥〳〴〵\［（{£¥"々〇〉》」＄｠￥￦ #
```

Implementation meaning:

- Prefer not to leave these characters at line end.

### Following characters

```text
)]｝〕〉》」』】〙〗〟’”｠»ヽヾーァィゥェォッャュョヮヵヶぁぃぅぇぉっゃゅょゎゕゖㇰㇱㇲㇳㇴㇵㇶㇷㇸㇹㇺㇻㇼㇽㇾㇿ々〻‐゠–〜?!‼⁇⁈⁉・、%,.:;。！？］）：；＝}¢°"†‡℃〆％，．
```

Verification note:

- `Leading Characters` and `Following Characters` come from `resources.assets` TextAssets.
- `Leading Characters` includes one real ASCII space before `#`.

Implementation meaning:

- Prefer not to start a line with these characters.

## Derived constraints and validation

### Usable area

- width: `300 - 10 - 10 = 280`
- height: `265 - 10 - 10 = 245`

`280 × 245` is the real layout budget. Single-character capacity observations are validation anchors, not the primary model.

### Why the limit is 9 lines

- Latin line height from `NotoSans-Regular` at `fontSize = 18` is about `24.516 px`
- `245 / 24.516 ≈ 9.99`, which yields `9`
- CJK line height from `NotoSansCJK-JP-Regular` at `fontSize = 18` is about `26.064 px`
- `245 / 26.064 ≈ 9.39`, which also yields `9`

The `9`-line cap is explained directly by text height, font size, and font metrics.

### Width anchors

- `x`: `advance = 529`, width at `18px` is about `9.522 px`, so `280 / 9.522 ≈ 29.40`, which yields `29`
- `中`: `advance = 1000`, width at `18px` is `18 px`, so `280 / 18 ≈ 15.55`, which yields `15`
- `.`: `advance = 268`, width at `18px` is about `4.824 px`, so `280 / 4.824 ≈ 58.04`, which yields `58`

ASCII punctuation such as `.`, `,`, `:`, and `;` stays narrow under `NotoSans-Regular`. It should not be treated like CJK full-width punctuation.

### Validation anchors

Key observed anchors are explained by the fixed model:

| Character | Measured | Predicted |
|---|---:|---:|
| `中` | `15` | `15` |
| `x` | `29` | `29` |
| `X` | `26` | `26` |
| `1` | `27` | `27` |
| `m` | `16` | `16` |
| `w` | `19` | `19` |
| `W` | `16` | `16` |
| `0` | `27` | `27` |
| `.` | `58` | `58` |
| `:` | `58` | `58` |
| `，` | `15` | `15` |

Additional confirmed validation:

- the later `a..z` and `A..H` sample set matched the model `34 / 34`
- `中 × 144` showing only `135` visible characters is explained by `15 × 9 = 135`

## Implementation rules

- Use real glyph widths, not a fixed character-count heuristic.
- Wrap against the usable width budget of `280 px`.
- Use grapheme clusters as the default processing boundary when simulating wrapping behavior.
- Determine legal break opportunities from Unicode line-break behavior plus TMP leading/following restrictions.
- Prefer legal break opportunities and fall back to the nearest legal break before hard clipping; if none exists, hard-break at a grapheme-cluster boundary.
- Spaces both consume width and act as break opportunities; if a wrap happens at a space, the next line should not keep that leading break-space.
- Continuous CJK text is breakable between characters by default, but breaks must still respect TMP leading/following restrictions.
- For complex scripts that depend on shaping or reordering, use shaped glyph advances rather than per-codepoint widths.
- Clip output to at most `9` visible lines after wrapping.

## Known unknowns

- The full custom MonoBehaviour typetree was not recovered, so some non-critical fields remain inferred rather than directly dumped.
- The short-text chat bubble background resize logic is still not the authoritative model. The inspected object chain did not expose clearly named `ContentSizeFitter`, `LayoutElement`, `HorizontalLayoutGroup`, or `VerticalLayoutGroup` components, which suggests the width change is likely driven by custom script logic. This does not affect long-text wrapping and clipping inside `ChatText`.
- Small non-critical field differences between normal and mirrored objects, including possible `overflowMode` differences, should not be used as primary implementation inputs unless re-verified.

## Verification appendix

Selected IDs for future spot-checking:

- `TMP Settings`: `resources.assets`, path id `107463`
- `LineBreaking Leading Characters` TextAsset: `1767`
- `LineBreaking Following Characters` TextAsset: `1784`
- primary TMP font asset `NotoSans-Regular SDF`: `sharedassets0.assets`, path id `6203`
- primary raw font `NotoSans-Regular`: `Font`, path id `925`
- primary CJK raw font `NotoSansCJK-JP-Regular`: `Font`, path id `923`
- chatbox material `NotoSans-Regular SDF Nameplates ChatBubble`: path id `103`
- SDF atlas `NotoSans-Regular SDF Atlas`: path id `494`
- normal `ChatText` component: `6792`
- mirrored `ChatText` component: `6471`
- normal `ChatText` RectTransform: `5914`
- mirrored `ChatText` RectTransform: `6022`
- normal `Chat` RectTransform: `5902`
- normal `Canvas` RectTransform: `6024`
- `ChatBubble` Transform: `4145`
- `ChatBubbleMirror` Transform: `4871`
