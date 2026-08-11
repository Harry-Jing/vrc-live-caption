import { expect, test } from "vitest";
import wireVocabularyFixture from "../../../contracts/wire-vocabulary-v2.json?raw";
import { CAPTION_LANES, CAPTION_STATES } from "../captionAggregate";
import {
  CAPTION_BOUNDARY_OWNERS,
  CAPTION_UNIT_BEHAVIORS,
  LANE_UPDATE_BEHAVIORS,
  PUBLICATION_INCOMPATIBILITY_REASONS,
  PUBLICATION_MODES,
  PUBLICATION_PLAN_STATES,
  RECOGNITION_INPUT_SHAPES,
  RECOGNITION_PATHS,
  RESOLVED_PUBLICATION_TIMINGS,
  REVISION_BEHAVIORS,
} from "../captionPipeline";
import {
  CHATBOX_PUBLICATION_STATES,
  CREDENTIAL_IDS,
  CREDENTIAL_STATUS_STATES,
  CREDENTIAL_STORAGES,
  RUNTIME_GENERATION_PHASES,
  RUNTIME_PENDING_GENERATION_CHANGES,
} from "../runtimeControl";
import {
  DIAGNOSTIC_CATEGORIES,
  DIAGNOSTIC_SEVERITIES,
  RUNTIME_STATUSES,
} from "../runtimeEvents";

test("closed frontend wire values match the shared vocabulary", () => {
  expect(JSON.parse(wireVocabularyFixture) as unknown).toEqual({
    runtimeStatuses: RUNTIME_STATUSES,
    credentialIds: CREDENTIAL_IDS,
    credentialStorages: CREDENTIAL_STORAGES,
    credentialStatusStates: CREDENTIAL_STATUS_STATES,
    diagnosticCategories: DIAGNOSTIC_CATEGORIES,
    diagnosticSeverities: DIAGNOSTIC_SEVERITIES,
    captionLanes: CAPTION_LANES,
    captionStates: CAPTION_STATES,
    publicationModes: PUBLICATION_MODES,
    recognitionPaths: RECOGNITION_PATHS,
    recognitionInputShapes: RECOGNITION_INPUT_SHAPES,
    captionBoundaryOwners: CAPTION_BOUNDARY_OWNERS,
    captionUnitBehaviors: CAPTION_UNIT_BEHAVIORS,
    laneUpdateBehaviors: LANE_UPDATE_BEHAVIORS,
    revisionBehaviors: REVISION_BEHAVIORS,
    resolvedPublicationTimings: RESOLVED_PUBLICATION_TIMINGS,
    publicationPlanStates: PUBLICATION_PLAN_STATES,
    publicationIncompatibilityReasons: PUBLICATION_INCOMPATIBILITY_REASONS,
    runtimePendingGenerationChanges: RUNTIME_PENDING_GENERATION_CHANGES,
    runtimeGenerationPhases: RUNTIME_GENERATION_PHASES,
    chatboxPublicationStates: CHATBOX_PUBLICATION_STATES,
  });
});
