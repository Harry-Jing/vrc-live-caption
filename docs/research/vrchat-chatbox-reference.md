# VRChat Chatbox Reference

This document is the canonical developer reference for VRChat Chatbox OSC,
text limits, wrapping, and glyph behavior used by VRC Live Caption. It combines
VRChat's public interface, read-only client-resource inspection, controlled
runtime measurements, and the matching public TextMesh Pro source baseline.

The findings were reviewed on 2026-08-13. Layout behavior is build-scoped:
VRChat may change assets or private runtime logic without changing the OSC
address.

## Developer summary

Here, a **Completed page** is one sendable, ordered slice of final caption text.
A **Live viewport** is one replacement message that keeps the newest useful
context from an ongoing caption.

- Send text with `/chatbox/input (text, true, false)`. The first Boolean sends
  immediately; the second suppresses the notification sound.
- VRChat documents a limit of 144 characters and 9 displayed lines. On the
  tested Windows client, the visible input was the first **144 UTF-16 code
  units**, not 144 Unicode scalar values or grapheme clusters.
- VRChat's cutoff is not Unicode-safe. It can split a surrogate pair, combining
  sequence, emoji modifier sequence, ZWJ sequence, or CRLF pair.
- VRChat does not paginate an oversized OSC string. A sender must produce every
  safe Completed page or Live viewport before sending it.
- At most 9 visual lines are shown. Soft wraps and explicit line breaks consume
  the same line budget.
- Wrapping is glyph-width dependent. The useful text width is 280 local TMP
  units, but the number of characters that fit varies greatly by glyph.
- The tested client uses a TextMesh Pro 3.0.6-compatible layout configuration,
  Noto Sans as its primary font, and an ordered 19-font fallback list.
- Asset configuration and public TMP source explain much of the behavior, but
  the private VRChat component and the exact truncation and line-cap code were
  not recovered. They must not be presented as verified implementation details.
- UDP send success is not display or delivery acknowledgement. Pacing and
  remote-observer behavior require runtime validation.

## Evidence and build scope

This report separates evidence from product policy:

| Class | Meaning |
|---|---|
| **Official** | A behavior documented by VRChat, Unity, or Unicode. |
| **Client asset** | Raw bytes, names, object relationships, or built-in fields read from a hash-pinned installed client. Names for custom TMP fields depend on the disclosed public schema. |
| **Runtime observed** | A result visible after a controlled OSC input on the tested client. |
| **Source baseline** | Behavior explained by public TMP 3.0.6 source or API; useful as a model, but not proof of VRChat's private machine code. |
| **Deterministic Unicode** | Counts or boundaries computed from the exact input under a named Unicode or indexing rule. |
| **Product contract** | A safety or publication guarantee chosen by VRC Live Caption, not behavior attributed to VRChat. |
| **Unknown** | A private implementation detail or untested profile that the evidence cannot identify. |

The layout measurements came from 178 individually sent cases on 2026-08-13.
Each case used `/chatbox/input (payload, true, false)` and was checked after the
client had settled. Case identifiers were not included in the payload. Payload
and image hashes, ordering, and image integrity were independently checked for
all 178 cases. One empty payload produced no bubble; the other 177 cases
displayed.

A VRChat log covering the measurement interval and the matching static install
metadata identified this profile:

| Property | Value | Evidence |
|---|---|---|
| VRChat release | `2026.3.1-1885-81193b80fa-Release` | correlated log and embedded PlayerSettings value |
| Steam build | `24702028` | Steam app manifest |
| Unity | `2022.3.22f2-DWR` | correlated log and serialized-file metadata |
| Platform | Windows Steam, `standalonewindows` | correlated log |
| XR state | `XR Device: None` (Desktop) | correlated log |
| Chat bubble scale / opacity | `1.5` / `1` | correlated log |
| Observer viewpoint | Not recorded | unknown |

The log correlation is separate from the captured observations, so the
viewpoint remains unknown. These measurements do not establish PCVR, Quest,
remote-observer, or mirrored-prefab behavior.

The static asset findings use the same installed release. The two principal
containers were pinned as follows:

| File | SHA-256 |
|---|---|
| `VRChat_Data/resources.assets` | `0d5d00afcf06349e7dd286a8c3209b45851b973104d9c0fd80bece6312e2a940` |
| `VRChat_Data/sharedassets0.assets` | `ee488660d8faabc1fe03f36f4757883568236790cb9a6ab0c95c01b3f9708e08` |

## OSC interface and pacing

### Official interface

VRChat's [OSC input reference](https://docs.vrchat.com/docs/osc-as-input-controller#chatbox)
defines:

- `/chatbox/input s b n`: `s` is the text; `b = true` sends immediately and
  `b = false` opens the keyboard; `n = false` suppresses the notification sound.
- `/chatbox/typing b`: toggles the typing indicator.
- Chatbox text is limited to 144 characters, and at most 9 lines are displayed,
  including explicit line breaks and word wrap.

VRChat's [OSC overview](https://docs.vrchat.com/docs/osc-overview) uses UDP port
`9000` for input by default. On the wire, `(text, true, false)` has the OSC type
tag string `,sTF`; under the [OSC 1.0 specification](https://opensoundcontrol.stanford.edu/spec-1_0.html),
`T` and `F` carry no argument bytes. VRChat added UTF-8 Chatbox input in
[VRChat 2022.4.1](https://docs.vrchat.com/docs/vrchat-202241).

The public reference does not define the Unicode unit counted by “character.”
The UTF-16 result below is therefore a tested-build observation, not an official
protocol promise.

An [official 2022 client update](https://ask.vrchat.com/t/developer-update-19-august-2022/12775)
states that the typing indicator hides after five seconds without input. A
runtime recheck in July 2026 reproduced that one-shot timeout. The available
evidence does not establish whether sending `/chatbox/typing true` again resets
the timer. Validate that refresh behavior before relying on it as a keepalive.

### Rate limiting

[VRChat 2026.2.1](https://docs.vrchat.com/docs/vrchat-202621) replaced the old
flat timeout with a leaky-bucket limiter. The release note allows five messages
within five seconds before another manual message must wait, and says auto-sent
messages do not contribute to that limit.

The current OSC documentation does not specify whether any particular
`/chatbox/input` Boolean combination is classified as manual or auto-sent.
Therefore `(text, true, false)` must not be described as unlimited.

A sustained numbered-message experiment in July 2026 produced this practical
boundary:

| Attempt interval | Observed result |
|---:|---|
| 200-500 ms | Sequence numbers were skipped quickly. |
| 800 ms | Initially worked, then skipped periodically. |
| 900 ms | A short run worked; a 100-message run began skipping near 41-42. |
| 1000 ms | 120 consecutive messages displayed without an observed skip. |

Use one second between sustained text-send attempts as a conservative
integration constraint. It is not a documented protocol minimum. The older
[1.5-second recommendation](https://ask.vrchat.com/t/developer-update-11-august-2022/12286)
described the pre-2026 cooldown and remains historical UX context only.

VRChat carries this OSC input over UDP and provides no Chatbox acknowledgement.
A successful socket send only proves that the local operating system accepted
the datagram. It does not prove that VRChat displayed or relayed the text.

## Runtime text behavior

### The input boundary is a 144-unit UTF-16 prefix

The tested client behaved as though it retained the first 144 UTF-16 code units
and discarded the rest:

| Probe | Runtime observation |
|---|---|
| 143 / 144 / 145 ASCII units | 144 and 145 produced the same Chatbox pixels; the 145th unit was absent. |
| 71 emoji plus `a` / 72 emoji / 72 emoji plus `a` | The 144- and 145-unit results were identical; the final `a` was absent. |
| Surrogate pair across the boundary | The pair was split and a replacement-like shape appeared. |
| Combining sequence across the boundary | The base remained and the combining mark was removed. |
| Emoji modifier across the boundary | The sequence was split and a replacement-like shape appeared. |
| ZWJ sequence across the boundary | The sequence was cut between components. |
| `CRLF` across the boundary | The prefix ended after CR; LF was discarded. |
| 288 / 289 ASCII units | Both displayed the same first 144-unit prefix. No pagination occurred. |

This is strong evidence for visible UTF-16 prefix truncation on this build. It
does not identify whether the cutoff occurs at OSC ingress, in VRChat's private
Chatbox preprocessing, or later in the rendering path.

For a sender, the consequence is unambiguous: every transmitted string must be
at most 144 UTF-16 code units before it reaches VRChat. Choose that boundary at
an extended grapheme-cluster boundary even though VRChat itself does not. If a
single grapheme exceeds 144 units, the sender must apply an explicit error,
drop, replacement, or split policy; lossless output, no split, and a 144-unit
maximum cannot all be satisfied at once.

### Explicit lines and control characters

[VRChat 2024.1.1](https://docs.vrchat.com/docs/vrchat-202411) officially
allowlisted newline characters for OSC Chatbox input and introduced the hard
nine-line limit. The exact controls below are runtime observations; they are
literal Unicode characters in the OSC string, not backslash escape notation:

| Input | Tested-build behavior |
|---|---|
| LF (`U+000A`) | Starts a new line. |
| CRLF | Starts one new line. |
| VT (`U+000B`) | Starts a new line. |
| LINE SEPARATOR (`U+2028`) | Starts a new line. |
| PARAGRAPH SEPARATOR (`U+2029`) | Starts a new line. |
| bare CR (`U+000D`) | Does not add a line; it resets horizontal position and later text overdraws the current line. |
| NEL (`U+0085`) | Does not add a line; the tested client displayed its missing-glyph marker. |
| literal `\n` | Displays a backslash and `n`; it is not a line break. |
| FORM FEED (`U+000C`) | Not measured. |

Leading LF preserves a blank first row. Consecutive LFs preserve internal blank
rows. A trailing LF does not add visible trailing height. These details matter
when predicting a page independently from its source context.

An empty payload produced no bubble. A spaces-only payload produced an empty
bubble, and a payload made only from eight LFs produced a tall empty bubble.

Nine visual lines are shown. A ten-line input displayed lines one through nine
while hiding the tenth line's text. The bubble background still grew to the
ten-line height. A mixed probe with soft wraps and explicit LFs behaved the same
way, proving that both consume one shared nine-line visibility budget.

The line cap must not be derived from the text rectangle's height alone. The
public TMP API exposes `maxVisibleLines`, which can explain this result, but the
private VRChat field or runtime assignment that selects 9 was not recovered.

### Width and soft wrapping

The ChatText rectangle is 300 by 265 local units with 10-unit margins on every
side, leaving a nominal 280 by 245 text area. The observed boundaries are
glyph-width dependent, not a fixed characters-per-line grid. The extracted TMP
configuration enables wrapping and kerning and supplies fallback and line-break
tables; public TMP source explains how those inputs can drive layout.

Fifteen repeated-glyph pairs produced exact adjacent one-line/wrap boundaries:

| Glyph | Maximum on one line | First count that wrapped |
|---|---:|---:|
| `x` | 29 | 30 |
| `W` | 16 | 17 |
| `.` | 58 | 59 |
| `中` | 15 | 16 |
| `X` | 26 | 27 |
| `1` | 27 | 28 |
| `m` | 16 | 17 |
| `w` | 19 | 20 |
| `0` | 27 | 28 |
| `:` | 58 | 59 |
| `é` | 27 | 28 |
| `’` | 88 | 89 |
| `“` | 43 | 44 |
| `—` | 15 | 16 |
| `，` | 15 | 16 |

These are regression anchors for this build, not a replacement for measuring
arbitrary text. Punctuation can also participate in line-break rules, so not
every capacity should be interpreted as a pure isolated advance measurement.

Additional break observations:

- ASCII space and NBSP produced the same two-line pixels in the tested probe.
  That equality does not distinguish a legal break from an emergency wrap.
- NNBSP remained on one line in its probe.
- ZWSP and soft hyphen were invisible and remained on one line in their probes.
- Word joiner, hyphen-minus, and non-breaking hyphen produced two lines, with
  different text placement.
- CJK opening and closing punctuation changed the measured break from UTF-16
  offset 15 to 14, consistent with the client's extracted line-break tables.

These results are compatible with tailored TMP line breaking. They do not prove
full conformance to the Unicode Line Breaking Algorithm.

### Normalization, graphemes, emoji, and bidirectional text

VRChat did not provide a visible normalization guarantee:

- Latin NFC and NFD probes looked similar but were pixel-different.
- Hangul NFD remained visibly decomposed into Jamo instead of being normalized
  to precomposed syllables.
- A CJK ideographic variation sequence displayed the normal base ideograph plus
  the ring-point marker at the variation-selector position; no distinct IVS
  glyph was confirmed.

If VRC Live Caption chooses an NFC policy, it must normalize before measuring
and sending and treat that as a product transformation, not client behavior.

An extended grapheme cluster is also not necessarily one VRChat glyph or one
unbreakable layout unit. On the tested client:

- text-presentation and emoji-presentation smileys rendered identically as a
  monochrome glyph;
- a skin-tone sequence displayed a base thumbs-up plus a separate tofu-like
  marker;
- technologist and family ZWJ sequences decomposed into component glyphs;
- regional-indicator flags displayed as letters rather than flag glyphs;
- keycaps displayed the base character plus a separate square;
- a tag flag displayed only its base flag;
- a technologist ZWJ sequence placed at the wrap edge split visually across
  lines.

The sender should still preserve extended grapheme clusters. That is a safer
product guarantee than VRChat's native behavior, not an attempt to reproduce
its unsafe splits.

Four mixed-direction pairs compared raw text with RLI/LRI...PDI isolates:
Arabic-first plus LTR, LTR-first plus Arabic, Hebrew plus a price, and Arabic
plus a URL. Each raw/isolate pair produced identical pixels in the fixed
Chatbox region. This only establishes that the tested isolates had no visible
effect in those inputs. It does not prove that isolates are always ignored or
that either visual order is linguistically correct.

## Client layout configuration

### Serialized ChatText templates

The current `resources.assets` contains four serialized ChatText templates:

```text
VRCPlayer     / NameplateContainer / ChatBubble       / Canvas / HeightOffsetRect / Chat / ChatText
VRCPlayer     / NameplateContainer / ChatBubbleMirror / Canvas / HeightOffsetRect / Chat / ChatText
VRCPlayer_New / NameplateContainer / ChatBubble       / Canvas / HeightOffsetRect / Chat / ChatText
VRCPlayer_New / NameplateContainer / ChatBubbleMirror / Canvas / HeightOffsetRect / Chat / ChatText
```

All four ChatText `RectTransform` objects use the same centered 300 by 265
rectangle. All four component payloads are identical after their GameObject
pointer.

The component is an obfuscated `Assembly-CSharp` type. A matching public TMP
3.0.6 schema decodes the first 536 of its 592 bytes as a coherent
`TextMeshProUGUI` base. The remaining 56 bytes are identical across the four
objects but have no recovered field names.

Important serialized base fields are:

| Field | Value |
|---|---:|
| font asset | `NotoSans-Regular SDF` |
| font size / base size | `18` / `18` |
| font weight | `400` (Regular in TMP 3.0.6) |
| auto sizing | off |
| auto-size range | `16` to `26` |
| horizontal / vertical alignment | center / middle |
| character, word, line, paragraph spacing | `0` |
| character width adjustment | `0` |
| word wrapping | on |
| word-wrapping ratio | `0.4` |
| overflow mode | serialized `0` (`Overflow` in TMP 3.0.6) |
| kerning | on |
| rich text | off |
| parse control-character escapes | off |
| serialized right-to-left flag | off |
| margins | `(10, 10, 10, 10)` |

Do not invent a serialized `textWrappingMode = Normal` field for this build;
the matching 3.0.6 base schema exposes the legacy word-wrapping Boolean. The
`0.4` ratio is a serialized TMP value, not a complete line-break specification.

These fields establish prefab configuration. They do not prove which of the
four branches a given player uses or whether the private component changes a
field after instantiation.

### Fonts and fallback order

The primary TMP font asset is `NotoSans-Regular SDF`. Its local fallback table
contains these 19 assets in order:

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

This order was decoded from the font asset's fallback field, not inferred from
nearby names. The TMP global fallback list is empty. Emoji, Arabic, and CJK-JP
are dynamic font assets backed by raw fonts; an empty serialized glyph table for
one of those assets does not mean that the script is unsupported.

The TMP Settings missing-glyph character is `U+2E30 RING POINT`. That matches
the repeated ring-shaped marker in missing-glyph probes. The fallback table
defines a search order, but asset inspection and observed pixels do not reveal
the actual font and atlas chosen for every glyph at runtime.

Additional CJK-KR, CJK-SC, and CJK-TC assets exist in the client, but they are
not entries in this primary font's local fallback list. Do not infer
locale-specific Chatbox font selection from their presence alone.

### Extracted line-break tables

TMP Settings directly references two line-break TextAssets. Their leading BOM
is omitted below. The `Leading Characters` resource contains a real ASCII space
immediately before `#`.

`Leading Characters` — prefer not to leave these at the end of a line:

```text
([｛〔〈《「『【〘〖〝‘“｟«$—…‥〳〴〵\［（{£¥"々〇〉》」＄｠￥￦ #
```

`Following Characters` — prefer not to start a line with these:

```text
)]｝〕〉》」』】〙〗〟’”｠»ヽヾーァィゥェォッャュョヮヵヶぁぃぅぇぉっゃゅょゎゕゖㇰㇱㇲㇳㇴㇵㇶㇷㇸㇹㇺㇻㇼㇽㇾㇿ々〻‐゠–〜?!‼⁇⁈⁉・、%,.:;。！？］）：；＝}¢°"†‡℃〆％，．
```

The serialized setting `useModernHangulLineBreakingRules` is false. These
tables are stronger inputs than handwritten punctuation categories, but they
still do not define the entire wrapping algorithm.

### What TMP 3.0.6 explains—and what it does not

The installed client's package inventory names `com.unity.textmeshpro@3.0.6`.
The corresponding [Unity package documentation](https://docs.unity3d.com/Packages/com.unity.textmeshpro%403.0/manual/index.html)
and a version-tagged [browsable mirror of the 3.0.6 UPM source](https://github.com/needle-mirror/com.unity.textmeshpro/tree/3.0.6/Scripts/Runtime)
explain plausible mechanisms for:

- local then global fallback lookup and dynamic glyph population;
- glyph-advance-based wrapping and the two line-break tables;
- CR horizontal-position reset and LF/VT/LS/PS line advance;
- hiding mesh content after `maxVisibleLines`.

The package files used for this source comparison came from Unity's local UPM
cache and were pinned by SHA-256:

| File | SHA-256 |
|---|---|
| `package.json` | `640c3c9ea8d7e5431bfefaccc70d85ea7aed204686d16a596d412286d2b9ba0b` |
| [`TMP_Text.cs`](https://github.com/needle-mirror/com.unity.textmeshpro/blob/3.0.6/Scripts/Runtime/TMP_Text.cs) | `2cfcb00a4464c48ca87d05c692c752087a2a8a96944d54b83606d7762f0c7806` |
| [`TMPro_UGUI_Private.cs`](https://github.com/needle-mirror/com.unity.textmeshpro/blob/3.0.6/Scripts/Runtime/TMPro_UGUI_Private.cs) | `1f8ba223bdd284bd0dfff1aefcf5988c872c9f6dd616673a8d4d58a9b2bf816a` |
| [`TMP_FontAssetUtilities.cs`](https://github.com/needle-mirror/com.unity.textmeshpro/blob/3.0.6/Scripts/Runtime/TMP_FontAssetUtilities.cs) | `fb2d13588d6ebacc34ba1fd32e75780d16f3df90347546482a9cc4bf1328cf60` |

This is a source baseline, not recovered VRChat private code. The Player assets
have stripped type trees, the derived class name is obfuscated, and ordinary
IL2CPP metadata recovery failed because the metadata header is transformed.
Consequently, the following remain unknown:

- where the 144-unit cutoff is applied;
- which private field or method supplies the value 9;
- how the bubble background is sized;
- whether VRChat performs text or bidi preprocessing before TMP;
- what the 56-byte derived-field tail represents.

## Glyph and language coverage

Font presence is not the same as language support. The tested runtime produced
these narrower results:

| Test group | Observation on the tested build |
|---|---|
| Natural caption samples | Text was visible without the ring-point marker for English, Spanish, French, German, Brazilian Portuguese, Italian, Turkish, Vietnamese, Indonesian, Polish, Russian, Ukrainian, Greek, Simplified and Traditional Chinese, Japanese, Korean, Hebrew, Arabic, Persian, Urdu, Hindi, Bengali, Tamil, and Thai. |
| Positive script probes | Armenian, Georgian, Gujarati, Gurmukhi, Kannada, Lao, Malayalam, Odia, Tamil, Telugu, and Tibetan produced non-ring glyphs. |
| Missing-glyph probes | Every tested scalar for Cherokee, Ethiopic, Khmer, Myanmar, Sinhala, Gothic, and generic Private Use Area sentinels produced the ring-point marker. |
| VRChat private-use control | `U+E040` produced a real VRCCustom glyph. |

These observations apply only to the tested strings and build. Most non-English
samples still require native review for character order, joining, mark
placement, shaping, word segmentation, and semantic correctness. In particular,
“no missing-glyph marker” is not evidence that Arabic, Hebrew, Indic, Thai, Lao,
or Tibetan layout is correct.

Complex emoji support is also partial despite the emoji fallback. A base emoji
may render while its modifier, ZWJ composition, flag composition, or keycap
composition does not. Model these cases conservatively and validate any
user-facing support claim with real-client observations.

## Integration requirements for VRC Live Caption

The following are project-side safety and fidelity rules derived from the
evidence above. They are intentionally stronger than VRChat's native behavior.

### Input preparation

1. Count the exact outgoing string in UTF-16 code units and keep it at or below
   144.
2. Select every cutoff at an extended grapheme-cluster boundary. The current
   lockfile resolves `unicode-segmentation` to 1.13.3, which implements Unicode
   17.0 text segmentation. Treat a dependency update as a behavior change and
   rerun the regression cases.
3. Define an explicit policy for a single grapheme larger than 144 units; never
   silently loop or split it by accident.
4. Do not assume that VRChat performs NFC normalization. If the product
   normalizes, do it before both layout and transmission.
5. Treat rich-text-looking content literally. The current ChatText has rich
   text disabled, and `<b>`, `<color>`, and unknown tags rendered as text.

### Layout and pagination

1. Wrap against 280 local units and no more than 9 visual lines.
2. Use measured glyph advances for the verified fonts. Do not use a fixed
   character count or treat narrow ASCII punctuation as full-width CJK text.
3. Use Unicode line-break opportunities as a baseline, then apply the extracted
   TMP leading/following restrictions. The current model uses
   `unicode-linebreak` 0.1.5: Unicode 15.0 data with the crate's documented
   `SA`-to-`AL` tailoring. This is product-model behavior, not evidence of
   VRChat's internal Unicode revision. Revalidate the corpus after changing the
   dependency or tailoring.
4. Prefer a legal break that fits. If none exists, break only at a safe grapheme
   boundary and account for that emergency behavior explicitly.
5. Re-layout each Completed page from start-of-text context. Preserve prepared
   text order and content across pages; do not rely on VRChat to continue an
   oversized string.
6. For Live output, send a latest-wins safe viewport. Do not send a raw
   oversized caption and expect VRChat to retain the newest suffix; it retains
   the old prefix on the tested build.
7. Preserve Unicode normalization and the verified CRLF, LF, VT, LINE SEPARATOR,
   and PARAGRAPH SEPARATOR controls. Before both layout and send, replace each
   bare CR, NEL, and FORM FEED with one ASCII space. Bare CR and NEL are unsafe
   to pass through because their observed rendering is not a normal line break;
   FORM FEED uses the same conservative product policy while its client behavior
   remains unknown.
8. Scripts requiring shaping or bidi reordering need shaped glyph advances.
   When the implementation cannot shape a grapheme confidently, reserve space
   conservatively rather than underestimating it.

### Publication and validation

- Share one process-wide text-send pacer across Completed and Live publishers.
  One second between sustained attempts is the current conservative boundary.
- Keep typing-indicator control separate from text-send pacing and stop it when
  publication activity stops. Do not depend on periodic `true` packets as a
  keepalive until their reset semantics have been measured.
- Treat every UDP send receipt as a local transport result, never a VRChat
  display receipt.
- Keep the 15 measured width pairs as regression anchors, but test natural,
  multilingual, control-character, long-token, and fallback cases as well.
- Validate actual publisher output—not only raw oversized source strings—in a
  running client. Completed pages and Live viewports have different product
  contracts and should be replayed separately.

## Known unknowns and revalidation

The following are not established by the current evidence:

- the private function that truncates to 144 UTF-16 units;
- the private mechanism behind the nine-line visibility cap;
- the active old/new and normal/mirrored prefab branch for each viewpoint;
- exact dynamic-font and atlas provenance for every runtime glyph;
- linguistic correctness of complex shaping and bidirectional layout;
- local-versus-remote, Desktop-versus-PCVR, and PC-versus-Quest equivalence;
- how different Chatbox scale settings affect world-space appearance;
- the display-duration and fade model for repeated OSC updates;
- whether `(text, true, false)` is classified as rate-limit-exempt auto-send;
- whether another `/chatbox/typing true` packet resets the five-second typing
  timeout;
- FORM FEED behavior.

After a VRChat update, retain the old build-scoped results and rerun the same
stable cases. Record the exact release, Steam build, Unity version, platform,
XR mode, observer viewpoint, Chatbox scale, OSC arguments, and capture timing.
Discover resources by hierarchy and names; Unity path IDs are not stable across
builds.

## References

- [VRChat: OSC as Input Controller](https://docs.vrchat.com/docs/osc-as-input-controller#chatbox)
- [VRChat: OSC overview](https://docs.vrchat.com/docs/osc-overview)
- [VRChat: OSC DIY](https://docs.vrchat.com/docs/osc-diy)
- [OSC 1.0 specification](https://opensoundcontrol.stanford.edu/spec-1_0.html)
- [VRChat 2024.1.1 release notes](https://docs.vrchat.com/docs/vrchat-202411)
- [VRChat 2022.4.1 release notes](https://docs.vrchat.com/docs/vrchat-202241)
- [VRChat 2026.2.1 release notes](https://docs.vrchat.com/docs/vrchat-202621)
- [VRChat developer update: 19 August 2022](https://ask.vrchat.com/t/developer-update-19-august-2022/12775)
- [VRChat developer update: 11 August 2022](https://ask.vrchat.com/t/developer-update-11-august-2022/12286)
- [Unity TextMesh Pro 3.0 API: `TMP_Text`](https://docs.unity.cn/Packages/com.unity.textmeshpro%403.0/api/TMPro.TMP_Text.html)
- [Unity TextMesh Pro 3.0 API: `TMP_FontAsset`](https://docs.unity.cn/Packages/com.unity.textmeshpro%403.0/api/TMPro.TMP_FontAsset.html)
- [Unity TextMesh Pro 3.0.6 package documentation](https://docs.unity3d.com/Packages/com.unity.textmeshpro%403.0/manual/index.html)
- [TextMesh Pro 3.0.6 UPM source mirror](https://github.com/needle-mirror/com.unity.textmeshpro/tree/3.0.6/Scripts/Runtime)
- [`unicode-segmentation` 1.13.3](https://docs.rs/unicode-segmentation/1.13.3/unicode_segmentation/)
- [`unicode-linebreak` 0.1.5](https://docs.rs/unicode-linebreak/0.1.5/unicode_linebreak/)
- [Unicode Standard Annex #29 revision 47: Text Segmentation, Unicode 17.0](https://www.unicode.org/reports/tr29/tr29-47.html)
- [Unicode Standard Annex #14 revision 49: Line Breaking, Unicode 15.0](https://www.unicode.org/reports/tr14/tr14-49.html)
