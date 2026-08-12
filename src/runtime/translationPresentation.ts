import type { TranslationEndpoint } from "./appConfig";
import type {
  CaptionAggregateSnapshot,
  CaptionSnapshot,
  SourceSnapshotRef,
  TranslationFailureReason,
  TranslationUnitSnapshot,
} from "./captionAggregate";
import type { ContentSelection, TranslationTarget } from "./captionPipeline";
import type {
  RuntimeGenerationSnapshot,
  RuntimeGenerationTranslationState,
} from "./runtimeControl";

export type TranslationPresentationCaption = Readonly<{
  text: string;
  language: string | null;
}>;

type TranslationPresentationUnitBase = Readonly<{
  sourceRef: SourceSnapshotRef;
  source: TranslationPresentationCaption | null;
}>;

export type TranslationPresentationUnit =
  | (TranslationPresentationUnitBase &
      Readonly<{
        state: "pending";
        translation: null;
        reasonCode: null;
      }>)
  | (TranslationPresentationUnitBase &
      Readonly<{
        state: "completed";
        translation: TranslationPresentationCaption;
        reasonCode: null;
      }>)
  | (TranslationPresentationUnitBase &
      Readonly<{
        state: "failed";
        translation: null;
        reasonCode: TranslationFailureReason;
      }>);

type ActiveTranslationPresentation = Readonly<{
  content: Exclude<ContentSelection, "sourceOnly">;
  target: TranslationTarget;
  endpointKind: TranslationEndpoint["kind"];
  units: readonly TranslationPresentationUnit[];
}>;

export type TranslationPresentation =
  | Readonly<{
      state: "inactive";
      content: "sourceOnly" | null;
      target: null;
      endpointKind: null;
      reasonCode: null;
      units: readonly [];
    }>
  | (ActiveTranslationPresentation &
      Readonly<{
        state: "active";
        reasonCode: null;
      }>)
  | (ActiveTranslationPresentation &
      Readonly<{
        state: "degraded";
        reasonCode: TranslationFailureReason;
      }>);

function inactivePresentation(
  content: "sourceOnly" | null,
): TranslationPresentation {
  return {
    state: "inactive",
    content,
    target: null,
    endpointKind: null,
    reasonCode: null,
    units: [],
  };
}

function sourceRefMatches(
  left: SourceSnapshotRef | null,
  right: SourceSnapshotRef,
) {
  return (
    left !== null &&
    left.generation === right.generation &&
    left.streamId === right.streamId &&
    left.unitId === right.unitId &&
    left.revision === right.revision
  );
}

function sourceMatchesRef(
  caption: CaptionSnapshot,
  sourceRef: SourceSnapshotRef,
) {
  return (
    caption.lane === "source" &&
    caption.state === "completed" &&
    caption.generation === sourceRef.generation &&
    caption.streamId === sourceRef.streamId &&
    caption.unitId === sourceRef.unitId &&
    caption.revision === sourceRef.revision
  );
}

function presentCaption(
  caption: CaptionSnapshot,
): TranslationPresentationCaption {
  return { text: caption.text, language: caption.language };
}

function presentUnit(
  outcome: TranslationUnitSnapshot,
  aggregate: CaptionAggregateSnapshot,
  content: Exclude<ContentSelection, "sourceOnly">,
): TranslationPresentationUnit | null {
  const source = aggregate.captions.find((caption) =>
    sourceMatchesRef(caption, outcome.sourceRef),
  );
  if (source === undefined) {
    return null;
  }

  const presentedSource =
    content === "bilingual" ? presentCaption(source) : null;

  switch (outcome.state) {
    case "pending":
      return {
        state: "pending",
        sourceRef: outcome.sourceRef,
        source: presentedSource,
        translation: null,
        reasonCode: null,
      };
    case "failed":
      return {
        state: "failed",
        sourceRef: outcome.sourceRef,
        source: presentedSource,
        translation: null,
        reasonCode: outcome.reasonCode,
      };
    case "completed": {
      const translation = aggregate.captions.find(
        (caption) =>
          caption.lane === "translation" &&
          caption.state === "completed" &&
          sourceRefMatches(caption.sourceRef, outcome.sourceRef),
      );

      return translation === undefined
        ? null
        : {
            state: "completed",
            sourceRef: outcome.sourceRef,
            source: presentedSource,
            translation: presentCaption(translation),
            reasonCode: null,
          };
    }
  }
}

export function translationPresentation(
  generation: RuntimeGenerationSnapshot | null,
  aggregate: CaptionAggregateSnapshot | null,
): TranslationPresentation {
  if (generation === null) {
    return inactivePresentation(null);
  }

  const content = generation.selection.publication.content;
  if (content === "sourceOnly") {
    return inactivePresentation(content);
  }

  const translation = generation.selection.translation;
  if (translation === null) {
    return inactivePresentation(null);
  }

  const activeStream = aggregate?.activeStream;
  const units =
    aggregate !== null &&
    activeStream !== null &&
    activeStream !== undefined &&
    activeStream.generation === generation.id
      ? aggregate.translationUnits.flatMap((outcome) => {
          if (
            outcome.sourceRef.generation !== generation.id ||
            outcome.sourceRef.streamId !== activeStream.streamId
          ) {
            return [];
          }

          const unit = presentUnit(outcome, aggregate, content);
          return unit === null ? [] : [unit];
        })
      : [];

  const runtimeState: RuntimeGenerationTranslationState =
    generation.translationState;
  return runtimeState.state === "degraded"
    ? {
        state: "degraded",
        content,
        target: translation.target,
        endpointKind: translation.endpoint.kind,
        reasonCode: runtimeState.reasonCode,
        units,
      }
    : {
        state: "active",
        content,
        target: translation.target,
        endpointKind: translation.endpoint.kind,
        reasonCode: null,
        units,
      };
}
