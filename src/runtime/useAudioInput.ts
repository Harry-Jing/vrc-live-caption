import { ref, shallowRef } from "vue";
import { uiText } from "../i18n/uiText";
import type { RuntimeBackend } from "./backend";
import type {
  AudioLevelEvent,
  AudioProbeRequest,
  AudioProbeResult,
} from "./types";

type AudioInputBackend = Pick<RuntimeBackend, "probeAudioInput">;

function probeErrorMessage(cause: unknown) {
  if (typeof cause === "string") {
    return cause;
  }
  if (cause instanceof Error) {
    return cause.message;
  }
  return uiText("settings.microphoneTest.unknownError");
}

export function useAudioInput(backend: AudioInputBackend) {
  const latestAudioLevel = shallowRef<AudioLevelEvent | null>(null);
  const audioProbeResult = shallowRef<AudioProbeResult | null>(null);
  const audioProbeError = ref("");
  const isAudioProbeRunning = ref(false);

  function acceptAudioLevel(event: AudioLevelEvent) {
    const current = latestAudioLevel.value;
    if (
      current !== null &&
      (event.generation < current.generation ||
        (event.generation === current.generation &&
          event.revision <= current.revision))
    ) {
      return false;
    }

    latestAudioLevel.value = event;
    return true;
  }

  async function probeAudioInput(request: AudioProbeRequest) {
    audioProbeError.value = "";
    audioProbeResult.value = null;
    isAudioProbeRunning.value = true;

    try {
      const result = await backend.probeAudioInput(request);
      audioProbeResult.value = result;
      return result;
    } catch (cause) {
      audioProbeError.value = probeErrorMessage(cause);
      return null;
    } finally {
      isAudioProbeRunning.value = false;
    }
  }

  return {
    latestAudioLevel,
    acceptAudioLevel,
    audioProbeResult,
    audioProbeError,
    isAudioProbeRunning,
    probeAudioInput,
  };
}
