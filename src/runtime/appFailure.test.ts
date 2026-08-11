import { describe, expect, test } from "vitest";
import { normalizeAppFailure } from "./appFailure";

describe("application failures", () => {
  test("preserves a structured application error code and message", () => {
    const cause = Object.assign(new Error("Microphone is busy."), {
      code: "audio.failed",
    });

    expect(normalizeAppFailure(cause, "Fallback failure.")).toEqual({
      code: "audio.failed",
      message: "Microphone is busy.",
    });
  });

  test.each([
    [new Error("Ordinary failure."), "Ordinary failure."],
    ["String failure.", "String failure."],
    [undefined, "Fallback failure."],
  ])(
    "normalizes unstructured failures without inventing a code",
    (cause, message) => {
      expect(normalizeAppFailure(cause, "Fallback failure.")).toEqual({
        code: null,
        message,
      });
    },
  );
});
