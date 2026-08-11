# Bilingual output is one asynchronous view

Date: 2026-07

Bilingual Chatbox messages render source above translation in a single
message. The measured Chatbox budget is shared flexibly rather than split
50/50, and spare capacity leans toward the translation: if you chose bilingual,
the translation is the part you are showing to other players. In Live, source
may run ahead of the translation so fresh text is never blocked by strict
sentence alignment; the app still keeps exact source/translation linkage
internally. The current layout evidence lives in the
[VRChat Chatbox reference](../research/vrchat-chatbox-reference.md).

Consequences: normal delay may leave the translation one unit behind. If
translation fails, the bilingual selection stays, the app shows a degraded
state, and stale translations are dropped rather than shown under newer
source text.

Revisit if observer testing shows loose alignment confuses more than
alignment latency would.
