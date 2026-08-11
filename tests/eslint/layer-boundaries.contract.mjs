// This fixture turns ESLint suppressions into negative contract tests.
// reportUnusedDisableDirectives makes lint fail if a boundary rule stops
// reporting one of the imports below.

// eslint-disable-next-line no-restricted-imports -- explicit TypeScript extensions must not expose runtime adapter implementations
import "../../src/platform/tauri/appGateway.ts";

// eslint-disable-next-line no-restricted-imports -- static imports must not expose Tauri APIs
import "@tauri-apps/api/core";

// eslint-disable-next-line no-restricted-imports -- UI modules must not import preview implementations
import "../../src/platform/preview/appGateway.ts";

// eslint-disable-next-line no-restricted-imports -- UI modules must not compose AppGateway adapters
import "../../src/platform/appGateway.ts";

// eslint-disable-next-line no-restricted-imports -- UI modules must not bypass the runtime context
import "../../src/runtime/gateway.ts";

// eslint-disable-next-line no-restricted-imports -- UI modules must not decode wire payloads directly
import "../../src/runtime/wire/runtimeEventContract.ts";

// eslint-disable-next-line no-restricted-syntax -- dynamic imports must not expose Tauri APIs
void import("@tauri-apps/api/core");

// eslint-disable-next-line no-restricted-syntax -- dynamic imports must not expose runtime adapter implementations
void import("../../src/platform/preview/appGateway");

// eslint-disable-next-line no-restricted-syntax -- dynamic imports must not expose runtime composition
void import("../../src/platform/appGateway");

// eslint-disable-next-line no-restricted-syntax -- static template imports must not expose Tauri APIs
void import(`@tauri-apps/api/event`);

// eslint-disable-next-line no-restricted-syntax -- static template imports must not expose runtime composition
void import(`../../src/runtime/gateway.ts`);

// eslint-disable-next-line no-restricted-syntax -- static template imports must not expose runtime adapter implementations
void import(`../../src/platform/tauri/appGateway.ts`);

// eslint-disable-next-line no-restricted-syntax -- static template imports must not expose runtime composition
void import(`../../src/platform/appGateway.ts`);

// eslint-disable-next-line no-restricted-syntax -- dynamic imports must not expose wire decoders
void import("../../src/runtime/wire/runtimeControlContract");

// eslint-disable-next-line no-restricted-syntax -- static template imports must not expose wire decoders
void import(`../../src/runtime/wire/captionAggregateContract.ts`);
