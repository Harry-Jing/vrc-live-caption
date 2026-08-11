import { ref, shallowRef } from "vue";
import { uiText } from "../../i18n/uiText";
import { normalizeAppFailure, type AppFailure } from "../appFailure";
import type { AppGateway } from "../gateway";
import type {
  AudioLevelEvent,
  AudioProbeRequest,
  AudioProbeResult,
} from "../audio";

type AudioInputGateway = Pick<AppGateway, "probeAudioInput">;

export function createAudioInputState(gateway: AudioInputGateway) {
  const latestAudioLevel = shallowRef<AudioLevelEvent | null>(null);
  const audioProbeResult = shallowRef<AudioProbeResult | null>(null);
  const audioProbeFailure = shallowRef<AppFailure | null>(null);
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
    audioProbeFailure.value = null;
    audioProbeResult.value = null;
    isAudioProbeRunning.value = true;

    try {
      const result = await gateway.probeAudioInput(request);
      audioProbeResult.value = result;
      return result;
    } catch (cause) {
      audioProbeFailure.value = normalizeAppFailure(
        cause,
        uiText("settings.microphoneTest.unknownError"),
      );
      return null;
    } finally {
      isAudioProbeRunning.value = false;
    }
  }

  return {
    latestAudioLevel,
    acceptAudioLevel,
    audioProbeResult,
    audioProbeFailure,
    isAudioProbeRunning,
    probeAudioInput,
  };
}
