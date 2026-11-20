# ADR: Trusted Lua core modules and plugin isolation

**Status:** Proposed; implementation blocked on a reviewed core-module inventory.

**Date:** 2026-11-20

## Context

The runtime currently loads every `.lua` file directly under `core_modules_dir` into a startup `LuaEnv`. That interpreter is not passed to `PluginExecutor`. Production plugin execution creates a fresh `PluginSandbox` VM for each request, with separate memory and instruction limits, a reduced standard library, bounded input/output, cancellation, and a narrow logging host capability.

This is an important isolation property. Passing the startup `Lua` instance into plugin execution would share mutable interpreter state across plugins and requests and is not an acceptable fix.

The repository search performed for this decision did not identify a committed core-module inventory. The configured default path is `./core`, but a configured path is not proof that trusted modules exist or are safe to expose.

## Decision

Preserve one fresh Lua VM per plugin execution. Do not share a mutable Lua interpreter between startup and plugin workers.

Before any core module is exposed to plugins:

1. Inventory every module by stable identifier, source path, owner, purpose, version, and SHA-256 digest.
2. Add an explicit reviewed registry/manifest. Files discovered in a directory are not automatically trusted or exposed.
3. Define a narrow module contract: trusted module source must return a module value; it must not depend on ambient filesystem, process, network, hardware, or dynamic-loader globals.
4. Make plugin-to-module grants explicit and separate from manifest capability requests. A plugin request is not a grant.
5. For each plugin execution, create the existing fresh sandbox, load only the exact reviewed modules granted to that plugin into that VM, and expose only immutable/read-only module interfaces. Do not provide a general `require` or filesystem-based module loader.
6. Keep module source immutable for the lifetime of an execution and verify its configured digest before use.
7. Fail closed if a requested module is missing, unreviewed, digest-mismatched, or incompatible. Do not silently skip a required module.
8. Keep module execution under the same per-VM memory, instruction, cancellation, wall-time, concurrency, and output bounds as plugin code. These are defense-in-depth controls, not an OS process boundary.

## Read-only interface requirements

A plain Lua table is not read-only: assigning to an existing key bypasses a simple `__newindex` guard. If modules expose tables, the runtime must use a proxy backed by private storage and recursively protect nested values, or expose only vetted functions and immutable scalar values. Tests must prove plugins cannot overwrite, shadow, or mutate trusted exports, including nested structures.

Trusted module functions must not capture mutable shared state across executions. If state is needed, it must be created inside the plugin's per-execution VM or mediated by a separately authenticated, capability-checked host service.

## Required tests before implementation can be accepted

- A plugin receives only modules explicitly granted to it.
- Unrequested modules are absent.
- Unknown, unreviewed, missing, or digest-mismatched modules fail closed.
- A plugin cannot overwrite or mutate trusted exports, including nested exports.
- One plugin cannot affect another plugin's module values or execution state.
- A failed or cancelled module initialization does not contaminate later executions.
- Module initialization obeys memory, instruction, wall-time, and cancellation limits.
- No dynamic loading, filesystem, shell, OS, debug, or network capability appears implicitly.
- Existing sandbox capability-denial and owner-isolation tests continue to pass.

## Consequences

The startup-only `LuaEnv` is not itself the plugin module registry. It should either be replaced by a source-validation/registry loader that stores immutable verified module definitions, or remain startup diagnostics only until that registry exists. The implementation must not create an illusion that modules are available to plugins when they are not.

No plugin-facing core module API should be enabled until the inventory, grant policy, immutable export mechanism, and tests are implemented together.
