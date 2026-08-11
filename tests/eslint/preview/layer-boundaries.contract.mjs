// Preview files have their own adapter scope. Unused suppressions make lint fail
// if static, dynamic, or zero-expression template imports escape that scope.

// eslint-disable-next-line no-restricted-imports -- Preview must not use Tauri APIs
import "@tauri-apps/api/core";

// eslint-disable-next-line no-restricted-imports -- Preview must not import the Tauri runtime adapter
import "../../../src/platform/tauri/appGateway.ts";

// eslint-disable-next-line no-restricted-imports -- Preview must not import runtime composition
import "../../../src/platform/appGateway.ts";

// eslint-disable-next-line no-restricted-imports -- Preview must not decode runtime wire payloads
import "../../../src/runtime/wire/runtimeEventContract.ts";

// eslint-disable-next-line no-restricted-syntax -- Preview dynamic imports must not use Tauri APIs
void import("@tauri-apps/api/event");

// eslint-disable-next-line no-restricted-syntax -- Preview dynamic imports must not expose the Tauri adapter
void import("../../../src/platform/tauri/appGateway");

// eslint-disable-next-line no-restricted-syntax -- Preview dynamic imports must not expose runtime composition
void import("../../../src/platform/appGateway");

// eslint-disable-next-line no-restricted-syntax -- Preview dynamic imports must not expose wire decoders
void import("../../../src/runtime/wire/runtimeControlContract");

// eslint-disable-next-line no-restricted-syntax -- Preview template imports must not use Tauri APIs
void import(`@tauri-apps/api/core`);

// eslint-disable-next-line no-restricted-syntax -- Preview template imports must not expose the Tauri adapter
void import(`../../../src/platform/tauri/appGateway.ts`);

// eslint-disable-next-line no-restricted-syntax -- Preview template imports must not expose runtime composition
void import(`../../../src/platform/appGateway.ts`);

// eslint-disable-next-line no-restricted-syntax -- Preview template imports must not expose wire decoders
void import(`../../../src/runtime/wire/captionAggregateContract.ts`);
