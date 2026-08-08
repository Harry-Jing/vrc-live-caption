import { expect, test } from "vitest";
import { renderComponent } from "../test/componentHarness";
import MicrophoneProbeControl from "./MicrophoneProbeControl.vue";

const baseProps = {
  disabled: false,
  error: "",
  isRunning: false,
  result: null,
  runtimeActive: false,
};

test("offers an enabled microphone test button", async () => {
  const html = await renderComponent(MicrophoneProbeControl, baseProps);

  expect(html).toContain("<button");
  expect(html).not.toMatch(/<button[^>]*\sdisabled(?:=|\s|>)/u);
  expect(html).toContain("Test microphone");
});

test("explains why microphone testing is unavailable during an active runtime", async () => {
  const html = await renderComponent(MicrophoneProbeControl, {
    ...baseProps,
    runtimeActive: true,
  });

  expect(html).toMatch(/<button[^>]*\sdisabled(?:=|\s|>)/u);
  expect(html).toContain("Stop the runtime before testing this microphone");
});

test("shows the pending microphone test state", async () => {
  const html = await renderComponent(MicrophoneProbeControl, {
    ...baseProps,
    isRunning: true,
  });

  expect(html).toMatch(/<button[^>]*\sdisabled(?:=|\s|>)/u);
  expect(html).toContain("Testing microphone…");
  expect(html).toContain('role="status"');
});

test.each([
  [
    "heard speech",
    { clipping: false, gateOpen: true },
    "Audio is above the speech threshold",
  ],
  [
    "a low signal",
    { clipping: false, gateOpen: false },
    "Audio is below the speech threshold",
  ],
  ["clipping", { clipping: true, gateOpen: true }, "Clipping detected"],
] as const)(
  "shows %s from the microphone result",
  async (_name, flags, status) => {
    const html = await renderComponent(MicrophoneProbeControl, {
      ...baseProps,
      result: {
        sampleRate: 48_000,
        durationMs: 2_000,
        rmsDbfs: -27.25,
        peakDbfs: -3.5,
        ...flags,
      },
    });

    expect(html).toContain(status);
    expect(html).toContain("RMS -27.3 dBFS · Peak -3.5 dBFS");
  },
);

test("presents a microphone probe failure as an alert", async () => {
  const html = await renderComponent(MicrophoneProbeControl, {
    ...baseProps,
    error: "Microphone is busy",
  });

  expect(html).toContain('role="alert"');
  expect(html).toContain("Microphone test failed");
  expect(html).toContain("Microphone is busy");
});
