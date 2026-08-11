# VRChat Chatbox Reference

This document is the canonical implementation reference for the fixed VRChat chatbox wrap model used by `VRC Live Caption`. It keeps the project-facing layout, font, validation, and line-break facts needed to simulate VRChat wrapping and clipping without the reverse-engineering narrative.

The OSC and rate-limit sections were re-verified against first-party VRChat
documentation on 2026-07-15. Layout extraction facts retain their original
verification basis.

## Overview

- The chatbox model is a fixed TMP text layout, not a configurable heuristic width model.
- Long-text behavior is defined by a fixed text rectangle, fixed margins, fixed font size, the VRChat font/fallback stack, real glyph widths, TMP line-break tables, and a hard `9`-line cap.
- For this project, the important target is text layout and clipping inside the `ChatText` rectangle, not full world-space chat bubble rendering.

## OSC input contract

These are the OSC endpoints VRChat exposes for the chatbox, separate from the
layout model below.

- `/chatbox/input s b n`: sets the chatbox text. `s` is the message, `b` true
  sends immediately (false opens the in-game keyboard instead), `n` true plays
  the notification sound for nearby players. This app sends
  `(text, true, false)`: immediate send, no notification sound, so captions do
  not ping other players on every update.
- The input text is hard-capped at `144` characters by VRChat. This cap applies
  before the layout model below: for Latin text it is reached well before the
  `9`-line cap (about `29` characters per line over `9` lines is far more than
  `144`), so `144` is the binding constraint for Latin output. For CJK,
  `15 × 9 = 135` visible characters stays under the cap.
- The project enforces the input cap conservatively as a UTF-16 budget and only
  cuts at grapheme boundaries. The official wording is `144` characters; this
  implementation rule avoids splitting surrogate pairs or visible clusters.
- `/chatbox/typing b`: toggles the typing indicator on the chat bubble. Useful
  for showing activity while speech is still being recognized.

The endpoint shape, `144`-character limit, and `9`-line limit are documented in
[VRChat's current OSC input reference](https://docs.vrchat.com/docs/osc-as-input-controller#chatbox).

### Typing indicator persistence

An [official 2022 client update](https://ask.vrchat.com/t/developer-update-19-august-2022/12775)
documented that the typing indicator automatically hides after five seconds of
inactive input. A July 2026 Phase 1 real-client test reproduced that behavior:
one `/chatbox/typing true` packet disappeared after about five seconds while the
speaker continued talking for roughly twenty seconds.

The Completed publisher therefore reasserts `/chatbox/typing true` every four
seconds while normalized speech or publication activity remains active. The
one-second margin covers ordinary scheduling delay. These control-state packets
do not pass through `ChatboxPacer` and do not consume a `/chatbox/input`
text-send opportunity. Activity resolution, failure, and Stop still turn the
indicator off; Stop permits only its one typing-off cleanup attempt.

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

### Project pacing policy

The reliable current-client boundary is therefore:

- keep at least `1000 ms` between actual text-send attempts;
- measure from the previous attempt, not from a drifting periodic timer;
- count a failed attempt too, so failure cannot create a rapid retry loop;
- do not exploit the initial burst allowance;
- coalesce Live revisions latest-wins instead of queueing missed intermediate
  screens;
- keep distinct Completed pages ordered in a bounded queue.

The current code uses one fixed `1000 ms` text-attempt interval for the lifetime
of the desktop process. Runtime output and OSC Test share the same pacing state,
failed attempts consume the next opportunity, and restarting Runtime does not
reset it. The removed legacy `osc.minIntervalMs` key is not migrated into the
supported V1 config. Archived pre-baseline configs instead load editable
defaults and require explicit review and save.

Cloud, model, or translation latency only makes the actual interval longer and
does not invalidate the one-second lower bound. Publication eligibility and
transport pacing remain independent: a timer can say that VRChat may be
updated, but cannot prove text is complete.

Future current-client validation should still use numbered messages and a
remote observer when available. A successful UDP send proves only that the
local socket accepted the packet, not that VRChat displayed or relayed it.

### Current Completed publisher policy

The current Phase 1 implementation publishes whole Completed caption units
through a dedicated non-blocking worker. Its queue behavior is:

- keep at most `32` resident pages that have not yet been sent successfully;
- expire a whole unit after `30` seconds only while it remains unstarted;
- define started publication at the unit's first actual text-send attempt, after
  it receives a pacing opportunity;
- on capacity pressure, remove the oldest whole unstarted units until the new
  unit fits, or reject the new whole unit if it cannot fit without splitting a
  unit or displacing one that has started;
- do not retry a failed page. The attempt still consumes the pacing opportunity,
  and the publisher discards that unit's failed and remaining pages, reports the
  failure, and may continue with later units;
- on Stop or a runtime-fatal close, close admission and discard every resident
  page without draining caption text, then attempt one typing-off cleanup.

The `32`-page and `30`-second values are internal provisional safety limits, not
user settings or settled product limits. Phase 1 real-machine VRChat validation
must measure backlog and readability and adjust them as needed.

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

- Use the verified Noto Sans and Noto CJK glyph advances for covered common
  Chinese ideographs, Basic Latin/Latin-1 English, measured punctuation, and
  mixtures of those characters, not a fixed character-count heuristic.
- Wrap against the usable width budget of `280 px`.
- Use grapheme clusters as the default processing boundary when simulating wrapping behavior.
- Determine legal break opportunities from Unicode line-break behavior plus TMP leading/following restrictions.
- Use soft wraps only to simulate visible lines and choose page boundaries. Do
  not insert artificial newlines into a page; preserve only the source's
  explicit line breaks.
- Pages are a lossless partition of the source. If an explicit line-break
  grapheme would create a tenth line, keep it at the start of the next page and
  count it there; do not consume or move it merely because it crosses a page
  boundary.
- Re-layout every candidate page as standalone text and shrink it until it still
  fits one page from start-of-text context. UAX or TMP state from a prior page
  must not make a page appear safer than it will be when sent independently.
- Prefer legal break opportunities and fall back to the nearest legal break
  before a page boundary; if none exists, hard-break at a grapheme-cluster
  boundary.
- Spaces both consume width and act as break opportunities; if a wrap happens at a space, the next line should not keep that leading break-space.
- If a zero-advance modifier joins a break-space into one grapheme, project the
  space's internal legal break to the end of that grapheme. Never split the
  grapheme merely to use the original UAX break position.
- Continuous CJK text is breakable between characters by default, but breaks must still respect TMP leading/following restrictions.
- In the current pure-layout stage, other Unicode text uses conservative
  best-effort advances while preserving grapheme clusters, content order, and
  every page limit. An unsupported grapheme reserves the full `280 px` line
  budget so an unknown wide glyph is not underestimated. This intentionally
  sparse fallback does not relax the product-wide target of real glyph-width
  wrapping; verified shaping and language-specific line breaking remain future
  quality work for those languages.
- If one grapheme alone exceeds the `144` UTF-16-unit budget, no compliant page
  can both preserve and avoid splitting it. Return an explicit layout error
  rather than splitting, dropping, or sending an over-limit grapheme.
- Limit each page to at most `9` visible lines after wrapping. The pure
  Completed layout returns all remaining text as later ordered pages instead of
  clipping it. The implemented Completed publisher consumes those pages in
  order; Live viewport and translation-aware rendering remain later stages.

## Known unknowns

- The chatbox display duration model is unverified: how long a message stays
  visible, whether a new `/chatbox/input` resets the timer, and when the bubble
  fades. These need in-game measurement before tuning replacement pacing.
- Whether OSC `(text, true, false)` is classified as exempt auto-sent text is
  not documented. The project does not depend on an exemption.
- The exact internal bucket implementation and what every excess update does
  are not documented. The project uses the measured 1000 ms boundary instead of
  relying on those internals.
- Local-sender and remote-observer behavior may differ and must be measured
  independently.
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
