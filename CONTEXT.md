# VRC Live Caption Glossary

VRC Live Caption turns speech into captions and translations for the VRChat
Chatbox. This glossary names the concepts shared across product, runtime, and UI
work without tying them to one provider or protocol.

## Recognition

**Speech recognition**:
The subsystem that turns microphone audio into normalized Source snapshots;
domain and code names use Recognition. `STT` remains user shorthand and the
stable `stt.*` diagnostic namespace.
_Avoid_: STT in new domain type or field names

**Recognition path**:
A complete speech-recognition choice, including the provider, model, protocol,
runtime, backend, and configuration that determine its behavior.
_Avoid_: Model, when the complete execution choice is intended

**Recognition Module**:
The application boundary that runs one selected recognition path for a runtime
generation and presents normalized recognition behavior to the rest of the app.
_Avoid_: Provider session, recognition worker

**Recognition Driver**:
The path-specific executor inside a Recognition Module.
_Avoid_: Adapter, when execution and lifecycle ownership are intended

**Runtime generation**:
One user-started captioning lifetime from Start through its matching Stop.
_Avoid_: Provider session, connection attempt

**Recognition attempt**:
One replaceable recognition execution inside a runtime generation, backed by a
cloud connection or local worker session.
_Avoid_: Runtime generation

**Translation path**:
A complete translation choice that produces a translation lane from correlated
source material.
_Avoid_: Translation model, when provider or runtime behavior is intended

## Captions

**Caption stream**:
The ordered correlation scope for captions inside one runtime generation; it may
span more than one recognition attempt.
_Avoid_: Provider stream, connection

**Caption unit**:
A correlated span of speech and its source and translation lanes.
_Avoid_: Sentence, because a boundary need not be grammatical

**Caption lane**:
An ordered sequence of source or translation snapshots.
_Avoid_: Transcript, when translated text is included

**Caption snapshot**:
The complete text of one caption lane at one revision.
_Avoid_: Delta

**Ongoing snapshot**:
A caption snapshot that may still be revised.
_Avoid_: Stable, provisional final, soft final

**Completed snapshot**:
The final snapshot in one caption lane's revision chain.
_Avoid_: Provider final, unless naming the provider's own event

**Caption Aggregate**:
The application-owned view of the active caption stream and recent normalized
caption state.
_Avoid_: Provider session, transcript history

**Source snapshot reference**:
The exact source snapshot consumed by a translation snapshot.
_Avoid_: Latest source, current caption

## Publication

**Chatbox publication mode**:
The user's timing choice: **Completed** publishes completed snapshots only;
**Live** may also publish ongoing snapshots.
_Avoid_: Content selection, streaming toggle

**Content selection**:
The lanes the user wants to publish: source, translation, or both.
_Avoid_: Publication mode

**Publication policy**:
The rule that combines selected lanes, publication timing, path capabilities,
and output-sink constraints.
_Avoid_: Provider output mode

**Caption Pipeline Plan**:
The resolved compatibility result for the selected recognition and translation
paths, content selection, publication mode, and output constraints.
_Avoid_: Backend plan

## Services and local inference

**Service provider**:
An external service identity, such as OpenAI, that may supply several paths.
_Avoid_: Recognition provider, when the service supplies other capabilities

**Service credential**:
An authentication identity for one service provider, potentially shared by
several paths.
_Avoid_: STT key, when the credential is service-wide

**Backend preference**:
The user's preferred compute backend for local inference.
_Avoid_: Effective backend

**Effective backend**:
The compute backend actually used by one local-inference attempt.
_Avoid_: Backend preference
