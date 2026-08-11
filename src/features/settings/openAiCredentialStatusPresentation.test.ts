import { expect, test } from "vitest";
import { openAiCredentialStatusPresentation } from "./openAiCredentialStatusPresentation";

test("presents an unavailable credential as an error instead of not saved", () => {
  expect(
    openAiCredentialStatusPresentation({
      state: "unavailable",
      id: "openai",
      failure: {
        code: "config.secret_failed",
        message: "Windows Credential Manager could not be opened.",
      },
    }),
  ).toEqual({
    label: "Unavailable",
    color: "error",
    failureMessage: "Windows Credential Manager could not be opened.",
    canRemove: false,
  });
});
