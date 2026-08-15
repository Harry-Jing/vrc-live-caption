import { describe, expect, test } from "vitest";
import apiBaseUrlFixtureJson from "../../contracts/translation-api-base-url-v1.json?raw";
import {
  translationApiBaseUrlNewEditValidationReason,
  translationApiBaseUrlV2ValidationError,
  type TranslationApiBaseUrlValidationReason,
} from "./appConfig";

type RejectedApiBaseUrlCase = Readonly<{
  name: string;
  url: string;
  reasonCode: TranslationApiBaseUrlValidationReason;
}>;

type ApiBaseUrlFixture = Readonly<{
  fixtureVersion: number;
  accepted: ReadonlyArray<Readonly<{ name: string; url: string }>>;
  rejected: ReadonlyArray<RejectedApiBaseUrlCase>;
  newEditRejectedV2Compatible: ReadonlyArray<RejectedApiBaseUrlCase>;
}>;

const apiBaseUrlFixture = JSON.parse(
  apiBaseUrlFixtureJson,
) as ApiBaseUrlFixture;

const expectedMessages = {
  invalidUrl: "API base URL must be a valid URL.",
  httpsRequired: "API base URL must use HTTPS.",
  hostRequired: "API base URL must include a host.",
  userInformationForbidden: "API base URL cannot contain user information.",
  queryOrFragmentForbidden: "API base URL cannot contain a query or fragment.",
  invalidPercentEncoding: "API base URL must contain valid percent encoding.",
  responsesEndpointForbidden:
    "API base URL cannot include the Responses endpoint.",
} satisfies Record<TranslationApiBaseUrlValidationReason, string>;

describe("App Config V2 Custom Translation API base URL validation", () => {
  test("consumes the versioned parity fixture", () => {
    expect(apiBaseUrlFixture.fixtureVersion).toBe(1);
  });

  test.each(apiBaseUrlFixture.accepted)("accepts $name", ({ url }) => {
    expect(translationApiBaseUrlV2ValidationError(url)).toBeNull();
  });

  test.each(apiBaseUrlFixture.newEditRejectedV2Compatible)(
    "preserves V2 compatibility for $name",
    ({ url }) => {
      expect(translationApiBaseUrlV2ValidationError(url)).toBeNull();
    },
  );

  test.each(apiBaseUrlFixture.rejected)(
    "rejects $name with its stable V2 message",
    ({ url, reasonCode }) => {
      expect(translationApiBaseUrlV2ValidationError(url)).toBe(
        expectedMessages[reasonCode],
      );
    },
  );
});

describe("new Custom Translation API base URL edit validation", () => {
  test.each(apiBaseUrlFixture.accepted)("accepts $name", ({ url }) => {
    expect(translationApiBaseUrlNewEditValidationReason(url)).toBeNull();
  });

  test.each([
    ...apiBaseUrlFixture.rejected,
    ...apiBaseUrlFixture.newEditRejectedV2Compatible,
  ])("rejects $name with a stable reason", ({ url, reasonCode }) => {
    expect(translationApiBaseUrlNewEditValidationReason(url)).toBe(reasonCode);
  });
});
