import { expect, test } from "vitest";
import { renderComponent } from "../test/componentHarness";
import LiveAudioMeter from "./LiveAudioMeter.vue";

const level = {
  generation: 7,
  revision: 3,
  rmsDbfs: -24.5,
  peakDbfs: -4.25,
  clipping: false,
  gateOpen: true,
  timestampMs: 1_000,
} as const;

test("shows an accessible live microphone reading for the active generation", async () => {
  const html = await renderComponent(LiveAudioMeter, {
    generation: 7,
    level,
    sessionPhase: "running",
  });

  expect(html).toContain('role="progressbar"');
  expect(html).toContain('aria-valuenow="-24.5"');
  expect(html).toContain('aria-valuemin="-96"');
  expect(html).toContain('aria-valuemax="0"');
  expect(html).toContain("RMS -24.5 dBFS · Peak -4.3 dBFS");
  expect(html).toContain("Speech gate open");
});

test("pauses during reconnect without presenting the old reading", async () => {
  const html = await renderComponent(LiveAudioMeter, {
    generation: 7,
    level,
    sessionPhase: "reconnecting",
  });

  expect(html).not.toContain('role="progressbar"');
  expect(html).toContain("Paused while reconnecting");
  expect(html).not.toContain("-24.5 dBFS");
});

test("does not show a stopped session's stale reading", async () => {
  const html = await renderComponent(LiveAudioMeter, {
    generation: null,
    level,
    sessionPhase: null,
  });

  expect(html).toBe("<!---->");
  expect(html).not.toContain('role="progressbar"');
});

test("does not reuse a prior generation's reading after restart", async () => {
  const html = await renderComponent(LiveAudioMeter, {
    generation: 8,
    level,
    sessionPhase: "running",
  });

  expect(html).not.toContain('role="progressbar"');
  expect(html).toContain("Waiting for microphone audio");
  expect(html).not.toContain("-24.5 dBFS");
});

test("announces clipping alongside the level", async () => {
  const html = await renderComponent(LiveAudioMeter, {
    generation: 7,
    level: { ...level, clipping: true },
    sessionPhase: "running",
  });

  expect(html).toContain('role="alert"');
  expect(html).toContain("Speech gate open");
  expect(html).toContain("Clipping detected");
});
