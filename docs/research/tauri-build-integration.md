# Tauri Build Integration

Reviewed on 2026-08-11. Project configuration and lockfiles are authoritative
for current commands, paths, and resolved versions.

## Build boundary

Frontend package scripts own Vite assets and Vue typechecking. The Tauri CLI
runs those scripts, compiles Rust, and creates bundles; Cargo owns only Rust.

```text
pnpm tauri dev
  -> beforeDevCommand: pnpm dev
  -> Vite at devUrl
  -> Tauri compiles and runs the Rust app

pnpm tauri build
  -> beforeBuildCommand: pnpm build
  -> vite build -> dist -> vue-tsc --build
  -> Tauri embeds frontendDist
  -> Cargo release build -> native bundles
```

Tauri documents `dev` as running `beforeDevCommand` and loading `devUrl`, while
`build` runs `beforeBuildCommand`, consumes `frontendDist`, and creates bundles
([`dev`](https://v2.tauri.app/reference/cli/#dev),
[`build`](https://v2.tauri.app/reference/cli/#build)).

## Hook execution

The full-app development path uses Tauri CLI; the frontend package's dev script
alone does not start Rust. `beforeDevCommand` must serve the same fixed endpoint
that `devUrl` names, so a frontend port change and Tauri configuration change
belong in one patch.

Direct Cargo or IDE launches do not run Tauri CLI hooks. Start the frontend
separately when deliberately debugging that way
([debugging guide](https://v2.tauri.app/develop/debug/vscode/#configure-launchjson)).

Both configured hooks are string commands. The reviewed CLI source runs them
from its resolved frontend directory, and resolves this repository root because
it contains `package.json` beside `src-tauri`
([hook runner](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/helpers/mod.rs#L668-L765),
[frontend resolver](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/helpers/app_paths.rs#L130-L166)).
The hooks therefore resolve the root package scripts without an explicit
`cwd`; revalidate this if either directory moves.

## Frontend declarations and typechecking

The frontend build deliberately runs Vite before `vue-tsc`. Vite transpiles
TypeScript but does not typecheck it
([Vite TypeScript guide](https://vite.dev/guide/features.html#typescript)).

Nuxt UI's Vite component plugin enables declaration generation by default when
component integration is enabled
([Nuxt UI 4.10.0 source](https://github.com/nuxt/ui/blob/v4.10.0/src/plugins/components.ts#L89-L135)).
Disabling composable auto-imports or local component scanning does not disable
those component declarations. The generated `components.d.ts` is ignored by
Git but included by the application TypeScript configuration.

Therefore Vite must initialize the Nuxt UI plugin and refresh declarations
before `vue-tsc` reads them on a clean checkout. Nuxt UI's official Vue example
also separates Vite build from `vue-tsc` and includes `components.d.ts`
([example scripts](https://github.com/nuxt/ui/blob/v4.10.0/playgrounds/vue/package.json#L6-L10),
[example tsconfig](https://github.com/nuxt/ui/blob/v4.10.0/playgrounds/vue/tsconfig.app.json#L25-L32)).
This order is a Nuxt UI integration constraint, not a Tauri constraint.

The shell ordering makes both stages one failing gate. In the reviewed Tauri CLI
source, `beforeBuildCommand` completes before `frontendDist` is checked, and a
non-zero hook status stops the build
([build setup](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/build.rs#L1075-L1112),
[hook runner](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/helpers/mod.rs#L668-L765)).
If typechecking fails after Vite writes `dist`, that Tauri invocation stops
before the Rust build and packaging phases.

## Assets, Rust, and packaging

`frontendDist: "../dist"` is relative to `src-tauri/tauri.conf.json` and points
to Vite's root-level output. Tauri recursively embeds a directory-form
`frontendDist` and uses its `index.html` as the default entry point
([Tauri configuration reference](https://v2.tauri.app/reference/config/#frontenddist)).

A Tauri build owns the frontend hook, asset validation, Cargo release build, and
native bundling. A direct Cargo build, check, or test does not run frontend
hooks, validate `dist`, or create Tauri bundles; it is a Rust-only check, not a
packaging check.

Supported local commands and quality gates live in
[CONTRIBUTING.md](../../CONTRIBUTING.md) and
[`package.json`](../../package.json). Do not duplicate their expansions here.

## CI ownership

The quality workflow checks frontend and Rust separately. The native-build
workflow is the integration and packaging gate: `tauri-action` invokes the
Tauri build path, so the frontend hook runs again before platform bundling. It
uploads build artifacts but does not create a project release. Workflow files
own the current matrices, arguments, and artifact paths. Tauri Action defines
its `args` as additional arguments to `tauri build`
([Tauri Action](https://github.com/tauri-apps/tauri-action#usage)).

## Revalidation

Re-run this investigation when a change can invalidate one of these boundaries:

- Tauri hook execution, frontend-root resolution, directory layout, or
  `frontendDist` ownership changes;
- Nuxt UI declaration generation, Vite/typecheck order, generated-file policy,
  or TypeScript inclusion changes;
- CI switches between separate checks and the Tauri packaging path, or changes
  how `tauri-action` passes build arguments.

Validate from a clean checkout where ignored declarations and `dist` are absent:
use the supported frontend gate, then the native Tauri build for the affected
platform. Record the review date and any changed assumption here.
