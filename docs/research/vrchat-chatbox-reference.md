# VRChat Chatbox Reference

This document is the canonical evidence reference for VRChat Chatbox OSC and
text-layout constraints used by `VRC Live Caption`. It records official
interfaces, first-party client notes, real-client experiments, and extracted
TMP layout/font data. Current publisher queues, lifecycle behavior,
configuration, and other implementation mechanics belong in code and tests,
not in this reference.

The OSC and rate-limit sections were re-verified against first-party VRChat
documentation on 2026-07-15. Layout extraction facts retain their original
verification basis.

## Overview

- The chatbox model is a fixed TMP text layout, not a configurable heuristic width model.
- Long-text behavior is defined by a fixed text rectangle, fixed margins, fixed font size, the VRChat font/fallback stack, real glyph widths, TMP line-break tables, and a hard `9`-line cap.
- For this project, the important target is text layout and clipping inside the `ChatText` rectangle, not full world-space chat bubble rendering.

## OSC and client-behavior evidence

### Official OSC surface

- `/chatbox/input s b n`: sets the chatbox text. `s` is the message, `b` true
  sends immediately (false opens the in-game keyboard instead), `n` true plays
  the notification sound for nearby players.
- Chatbox input is limited to `144` characters and `9` lines. The official
  reference does not define which Unicode counting unit “character” means.
- `/chatbox/typing b`: toggles the typing indicator on the chat bubble. Useful
  for showing activity while speech is still being recognized.

The endpoint shape, `144`-character limit, and `9`-line limit are documented in
[VRChat's current OSC input reference](https://docs.vrchat.com/docs/osc-as-input-controller#chatbox).

### Typing indicator persistence

An [official 2022 client update](https://ask.vrchat.com/t/developer-update-19-august-2022/12775)
documented that the typing indicator automatically hides after five seconds of
inactive input. A July 2026 real-client test reproduced that behavior:
one `/chatbox/typing true` packet disappeared after about five seconds while the
speaker continued talking for roughly twenty seconds.

### Current rate-limit evidence

VRChat 2026.2.1 removed the old flat Chatbox timeout and introduced a leaky
bucket. The release note says users may send five messages within five seconds
before the next message must wait. It also says auto-sent messages do not count
toward that limit and only manually sent messages are limited. See
[VRChat 2026.2.1](https://docs.vrchat.com/docs/vrchat-202621).
The later live [2026.2.2](https://docs.vrchat.com/docs/vrchat-202622) and
[2026.2.3](https://docs.vrchat.com/docs/vrchat-202623) notes do not document a
subsequent Chatbox rate-limit change as of this verification date.

This is not a documented fixed minimum interval per message. In particular,
`1.5` seconds is not a current hard protocol limit. That number came from a
[2022 official development update](https://ask.vrchat.com/t/developer-update-11-august-2022/12286),
which recommended updating every 1.5 seconds under the older cooldown behavior
and two-second minimum display setting. It remains useful historical UX
context, not the current rate-limit contract.

VRChat's current OSC reference only guarantees that `n = false` suppresses the
notification sound. Earlier official material called this field
`MessageComplete`, and Chatbox 2.0 later added live auto-send while typing, so
`n = false` may map to the exempt auto-sent category. No current official OSC
document makes that mapping a contract. The project must therefore not claim
that `(text, true, false)` has unlimited update rate. The continuous-send
experiment below also shows that sustained sub-second updates are not reliable
on the tested client.

### Project continuous-send experiment

A real-client numbered-message test was completed in July 2026 with
`/chatbox/input (text, true, false)`:

| Cadence | Observed result |
|---:|---|
| 200, 250, 500 ms | skipped sequence numbers quickly |
| 800 ms | initially worked, then skipped periodically under sustained sending |
| 900 ms | 40 messages appeared successful; a 100-message run began skipping near message 41-42 |
| 1000 ms | 120 consecutive messages without an observed skip |

The pattern is strongly consistent with an initial bucket of about five
messages and recovery of about one message per second. It is experimental
evidence, not a documented OSC protocol guarantee. In particular, a short 900
ms test can look successful because it consumes the initial allowance slowly.

### Derived pacing constraint

On the tested July 2026 client, `1000 ms` between sustained numbered sends was
the reliable conservative boundary. This is an experimental integration
constraint, not a documented fixed minimum interval and not a statement about
the current publisher implementation.

Future current-client validation should still use numbered messages and a
remote observer when available. A successful UDP send proves only that the
local socket accepted the packet, not that VRChat displayed or relayed it.

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

- the `a..z` and `A..H` sample set matched the model `34 / 34`
- `中 × 144` showing only `135` visible characters is explained by `15 × 9 = 135`

## Implementation rules

- Use real glyph widths from the verified font/fallback model, not a fixed
  character-count heuristic. Scripts that require shaping or reordering need
  shaped glyph advances rather than per-codepoint widths.
- Wrap against the usable width budget of `280 px`.
- Preserve grapheme clusters as the text-processing boundary.
- Determine legal wrap opportunities from Unicode line-breaking behavior plus
  the extracted TMP leading/following tables.
- Prefer legal break opportunities and fall back to the nearest legal break
  before a boundary; if none exists, hard-break at a grapheme-cluster boundary.
- Spaces consume width and act as break opportunities; when wrapping at a
  space, do not retain that leading break-space on the next line.
- Continuous CJK text is breakable between characters by default, while still
  respecting the TMP leading/following restrictions.
- Treat the official `144`-character input cap conservatively as a `144`
  UTF-16-unit safety budget, and never satisfy that budget by splitting a
  grapheme. This convention does not claim that VRChat documents UTF-16
  counting.
- Keep every transmitted view within `9` visible lines after wrapping.

## Known unknowns

- The chatbox display duration model is unverified: how long a message stays
  visible, whether a new `/chatbox/input` resets the timer, and when the bubble
  fades. These need in-game measurement before tuning replacement pacing.
- Whether OSC `(text, true, false)` is classified as exempt auto-sent text is
  not documented.
- The exact internal bucket implementation and what every excess update does
  are not documented; the numbered-send experiment does not resolve those
  internals.
- The official `144`-character limit does not define whether VRChat counts
  Unicode scalar values, UTF-16 code units, grapheme clusters, or another unit.
- Local-sender and remote-observer behavior may differ and must be measured
  independently.
- The full custom MonoBehaviour typetree was not recovered, so some non-critical fields remain inferred rather than directly dumped.
- Short-text bubble background sizing remains unverified. The inspected object
  chain did not expose `ContentSizeFitter`, `LayoutElement`,
  `HorizontalLayoutGroup`, or `VerticalLayoutGroup`, so custom script logic
  remains the leading explanation. This does not affect long-text wrapping and
  clipping inside `ChatText`.
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
