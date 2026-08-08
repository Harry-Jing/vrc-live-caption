import { expect, test } from "vitest";
import { createDecoders } from "./contractDecoding";

class TestContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`${path}: ${expectation}`);
  }
}

const decoders = createDecoders(TestContractError);

test("rejects unknown record fields with the supplied error type", () => {
  expect(() =>
    decoders.exactRecord({ expected: true, extra: true }, "$", ["expected"]),
  ).toThrow(TestContractError);
});

test("rejects non-finite numbers", () => {
  expect(() => decoders.finiteNumber(Number.NaN, "$.value")).toThrow(
    TestContractError,
  );
});

test("enforces an explicit safe-integer upper bound", () => {
  expect(() => decoders.safeInteger(65_536, "$.port", 0, 65_535)).toThrow(
    /0 to 65535/,
  );
  expect(decoders.safeInteger(65_535, "$.port", 0, 65_535)).toBe(65_535);
});
