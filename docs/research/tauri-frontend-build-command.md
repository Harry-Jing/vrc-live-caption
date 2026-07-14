# Tauri Frontend Build Command

Research completed on 2026-07-13 for this project's locked
`@tauri-apps/cli` version, `2.11.4`.

## Conclusion

For this project, the following package script is compatible with Tauri 2 and
is the correct order for Nuxt UI's generated TypeScript declarations:

```json
"build": "vite build && vue-tsc --build"
```

No Tauri configuration change is required. Keeping
`beforeBuildCommand: "pnpm build"` and `frontendDist: "../dist"` follows
Tauri's official Vite setup, which delegates frontend work to the package
`build` script and points Tauri at Vite's output directory
([Tauri Vite guide](https://v2.tauri.app/start/frontend/vite/#update-tauri-configuration)).

The ordering is a Nuxt UI requirement, not a Tauri requirement. Tauri waits
for the complete hook command and only cares that it succeeds and leaves a
valid frontend output directory. Nuxt UI's Vite plugin generates
`auto-imports.d.ts` and `components.d.ts`, and its official Vue starter runs
Vite build before typecheck
([Nuxt UI Vue installation](https://ui.nuxt.com/docs/getting-started/installation/vue#add-the-nuxt-ui-vite-plugin-in-your-viteconfigts),
[starter CI](https://github.com/nuxt-ui-templates/starter-vue/blob/73439bd669c7f8e8e7a0d8383c3f00917b789317/.github/workflows/ci.yml#L33-L40)).

## Official Tauri Guarantees

### Hook lifecycle and failure behavior

`beforeBuildCommand` is defined as a shell command that runs before
`tauri build` begins
([Tauri configuration reference](https://v2.tauri.app/reference/config/#beforebuildcommand)).
In the CLI version locked by this project, build setup invokes the hook before
checking `frontendDist` and before starting the Rust application build
([Tauri CLI 2.11.4 build source](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/build.rs#L188-L209)).

The hook is synchronous. Tauri runs it through `cmd /S /C` on Windows or
`sh -c` elsewhere, inspects its exit status, and returns an error for any
non-zero result
([Tauri CLI 2.11.4 hook source](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/helpers/mod.rs#L71-L126)).
Consequently, if `vue-tsc --build` fails after Vite has produced `dist`, the
overall `pnpm build` hook fails and Tauri does not continue to the Rust build
or packaging phase. A partially or fully generated local `dist` directory is
not packaged from that failed invocation.

### Working directory

For a string hook such as `"pnpm build"`, Tauri defaults the hook working
directory to its resolved frontend directory; an object-form hook can provide
an explicit `cwd`
([configuration type](https://v2.tauri.app/reference/config/#hookcommand),
[hook implementation](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/helpers/mod.rs#L78-L108)).
The frontend resolver treats a directory containing `package.json` as the
frontend root and otherwise falls back to the parent of `src-tauri`
([frontend directory resolver](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-cli/src/helpers/app_paths.rs#L130-L166)).
With this repository's conventional root `package.json` plus root
`src-tauri/`, the command therefore runs from the repository root. An explicit
`cwd` is unnecessary.

### Frontend output

Tauri resolves a relative `frontendDist` from the Tauri configuration file,
embeds the directory recursively, and expects `index.html` as the default
entry point
([Tauri configuration reference](https://v2.tauri.app/reference/config/#frontenddist)).
The official Vite integration specifically recommends `../dist` when
`tauri.conf.json` is under `src-tauri`
([Tauri Vite checklist](https://v2.tauri.app/start/frontend/vite/#checklist)).
This matches the current project layout and Vite's default output.

## Nuxt UI-Specific Ordering

The official Nuxt UI Vue setup says that the Vite plugin generates
`auto-imports.d.ts` and `components.d.ts`, that both files should be included
by the application TypeScript configuration, and that `#build/ui` aliases are
needed for theme type resolution
([Nuxt UI Vue installation](https://ui.nuxt.com/docs/getting-started/installation/vue#add-the-nuxt-ui-vite-plugin-in-your-viteconfigts)).
The official Vue starter separates `vite build` and `vue-tsc`, then executes
Build before Typecheck in CI
([starter scripts](https://github.com/nuxt-ui-templates/starter-vue/blob/73439bd669c7f8e8e7a0d8383c3f00917b789317/package.json#L5-L10),
[starter CI](https://github.com/nuxt-ui-templates/starter-vue/blob/73439bd669c7f8e8e7a0d8383c3f00917b789317/.github/workflows/ci.yml#L33-L40)).

Therefore, running `vue-tsc` before Vite on a clean checkout can typecheck
without the generated global-component declarations. Running Vite first makes
those declarations available to the immediately following `vue-tsc` command.
This is fully inside the package script; Tauri neither observes nor constrains
that internal order.

## Community Evidence and Known Edges

The following items are operational experience from Tauri's official GitHub
repositories, not API guarantees:

- A Tauri v2 community build log shows the common pattern of running a combined
  `tsc && vite build` package script through `beforeBuildCommand`; Tauri treats
  the combination as one frontend hook
  ([Tauri discussion #9419](https://github.com/tauri-apps/tauri/discussions/9419)).
  This supports combining typecheck and asset generation, but does not mandate
  their order.
- When asked whether CI should remove the hook after building the frontend in a
  separate step, a Tauri maintainer recommended keeping
  `beforeBuildCommand`; otherwise every local `tauri build` requires a manual
  frontend build first
  ([Tauri discussion #11810](https://github.com/orgs/tauri-apps/discussions/11810#discussioncomment-11402043)).
  This is maintainer guidance rather than a configuration guarantee.
- A reported nonstandard split layout exposed frontend-directory discovery
  problems in older CLI behavior
  ([Tauri issue #10417](https://github.com/tauri-apps/tauri/issues/10417)).
  The current hook supports explicit `cwd`, while this project's conventional
  layout does not need it.
- CI must install the package manager named by the hook. A Tauri Action issue
  shows a missing `bun` command causing `beforeBuildCommand` to exit non-zero
  and the build to stop
  ([tauri-action issue #986](https://github.com/tauri-apps/tauri-action/issues/986)).
  This project already standardizes on pnpm and installs dependencies before
  its build checks.

No Tauri-specific incompatibility with `vite build && vue-tsc --build` was
found in the official contract or the reviewed official GitHub reports. The
material risk is instead omitting Nuxt UI's generated declarations from a
clean typecheck.

One operational boundary remains: directly running `cargo build` does not
invoke Tauri CLI hooks. Use `pnpm tauri build` for a packaged application, or
run the frontend command separately when debugging through Cargo or an IDE
([Tauri VS Code debugging guide](https://v2.tauri.app/develop/debug/vscode/)).

## Project Recommendation

Use this structure:

1. Enable Nuxt UI declaration generation.
2. Include the generated declarations and `#build/ui` aliases in the relevant
   TypeScript configurations.
3. Keep generated declaration files ignored rather than committed.
4. Run `vite build && vue-tsc --build` as `pnpm build`.
5. Keep Tauri's `beforeBuildCommand` as `pnpm build` and `frontendDist` as
   `../dist`.

This preserves one failing frontend gate for direct builds, CI, and Tauri
packaging while ensuring Nuxt UI component props and slots are checked on a
clean checkout.
