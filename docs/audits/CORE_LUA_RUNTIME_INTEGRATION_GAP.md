# Janus Core Lua Runtime Gap — Stage 1 Finding

The current startup path creates `LuaEnv`, calls `load_core_modules`, and then constructs the API server. The `LuaEnv` instance is not passed into `PluginExecutor` or `PluginSandbox`.

Production plugin execution creates a fresh, separate Lua VM per request. Therefore, core Lua modules loaded into the startup `LuaEnv` are not visible to production plugin executions through the inspected code path.

This may be intentional isolation, but it means the current code should not claim that the configured core Lua modules are part of the production plugin runtime unless an explicit, tested interface connects them.

## Required design decision

Choose one of these explicit models before extending the module system:

1. **Trusted core library:** Load reviewed, administrator-managed core modules into each sandbox through a controlled, versioned, read-only API. Keep shell, filesystem, network, ADB, and hardware capabilities unavailable unless separately authorized and implemented.
2. **No runtime dependency:** Remove or clearly label startup-only core module loading if those files are not intended to be used by production plugins.
3. **Separate privileged service:** Expose specific core functions through authenticated runtime APIs with explicit capability checks, audit records, and bounded inputs/outputs.

Do not share a mutable Lua VM across concurrent plugin requests merely to make core functions visible.

## Acceptance tests

- A fixture core module exports one documented safe function.
- A production plugin can call it only through the chosen documented interface.
- A plugin cannot access unapproved globals, host filesystem, shell, network, ADB, or hardware.
- Core module load failure has deterministic startup behavior and an observable diagnostic.
- Core module versions/hashes are recorded so plugin execution is reproducible.
- Concurrent plugin requests cannot mutate shared interpreter state.

Until this is resolved, classify core Lua startup loading as **implemented but not integrated into production plugin execution**.
