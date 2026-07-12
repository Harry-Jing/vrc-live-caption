// This fixture turns ESLint suppressions into negative contract tests.
// reportUnusedDisableDirectives makes lint fail if a boundary rule stops
// reporting one of the imports below.

// eslint-disable-next-line no-restricted-imports -- explicit TypeScript extensions must not expose runtime backends
import "../../src/runtime/tauriBackend.ts";

// eslint-disable-next-line no-restricted-syntax -- dynamic imports must not expose Tauri APIs
void import("@tauri-apps/api/core");

// eslint-disable-next-line no-restricted-syntax -- dynamic imports must not expose runtime backends
void import("../../src/runtime/previewBackend");

// eslint-disable-next-line no-restricted-syntax -- static template imports must not expose Tauri APIs
void import(`@tauri-apps/api/event`);

// eslint-disable-next-line no-restricted-syntax -- static template imports must not expose runtime backends
void import(`../../src/runtime/backend.ts`);
