export type AudioInputDevice = Readonly<{
  id: string;
  name: string;
  isDefault: boolean;
}>;

export type AudioLevelEvent = Readonly<{
  generation: number;
  revision: number;
  rmsDbfs: number;
  peakDbfs: number;
  clipping: boolean;
  gateOpen: boolean;
  timestampMs: number;
}>;

export type AudioProbeRequest = Readonly<{
  inputDeviceId: string | null;
  durationMs: number;
}>;

export type AudioProbeResult = Readonly<{
  sampleRate: number;
  durationMs: number;
  rmsDbfs: number;
  peakDbfs: number;
  clipping: boolean;
  gateOpen: boolean;
}>;
