# claviron-php-embed

## Boundary

This crate owns in-process PHP execution through one managed ZTS SAPI.
It depends on `claviron-php-core` for route declarations and prepared runtime
values. It depends on neither the PHP facade, FCGI, Edge nor a listener.

The optional `backend` feature implements `claviron-pipeline` request owners.
The facade prepares those owners from site directives. Request policy and body
guards belong to the serving pipeline; the engine consumes the admitted body.

## Sources

- `lib.rs`: typed API and configured/native registration.
- `config/`: request pools, named applications, launch definitions and validation.
- `service/`: script, application and invoke execution lifecycles.
- `engine/`: PHP threads, request/application execution, body adaptation and interruption.
- `sapi/`: owned SAPI callbacks and module setup.
- `backend/`: script and persistent-application pipeline owners.
- `routing/`: native script/application request adaptation and script resolution.
- `extension.rs`, `observer.rs`, `framework/`: native extensions and framework registration.
- `functions/`: selected native PHP bridges.
- `event/`: PHP event delivery and owned value conversion.
- `intercept/`, `invalidation/`: OPcache/APCu hooks and fleet invalidation.
- `evaluate/`: returned-value execution without HTTP or provider boot.
- `console.rs`: caller-owned terminal PHP execution.
- `metrics/`: engine-specific local/fleet counters and formatting.
- `manager.rs`: application management.
- `php/warmup.php`: engine-owned warmup source.
- `tests/`: pure contracts and explicit native integration fixtures.

## Execution ownership

`PhpEmbedRuntime` owns shared process SAPI preparation. Script and persistent
application services retain their own execution generation, cancellation and
drain. A service may interrupt only its own running work, not another service's
threads or an idle request thread.

Request-scoped scripts receive a fresh native request scope. Persistent
applications retain their booted callable and isolate each request's data.
Task work uses its own owned execution placement and output. No mode creates
an HTTP listener.

PHP values, objects and resources cannot outlive their native request scope.
Owned values cross the Rust boundary only through the explicit conversion
contracts. Source and bootstrap inputs are trusted code, not a sandbox.

## Backend handoff

`backend::prepare_script` and `prepare_application` receive a prepared
`PhpRuntime`. They declare their identity and response-lifetime needs through
`BackendContract`; Pipeline performs the common handoff. Unhandled transfers
the original unread request. A response is terminal regardless of status.

Pinned entry scripts do not gain per-request existence/freshness probes.
[Core's execution IO contract](../core/ARCHITECTURE.md#execution_io-opcache_owns_freshness)
defines when mixed script/static directories may require a filesystem probe.

## Optional bridges

Database bridges lease configured pools per work unit. Explicit transactions
retain their lease until completion or work cleanup; cleanup rolls back
unfinished transactions and releases connection-local state. SQLx's own
backend errors and configured budgets remain authoritative.

Cache, auth, logging, event and database functions are selected by features and
prepared host bindings. PHP declarations in the Composer package do not install
a native bridge or emulate a missing service.

## Native checks

Pure tests do not start PHP, boot services or depend on deployment files.
`native_registration` is an explicit integration target with its own native
fixture. Native lifecycle checks use the configured ZTS toolchain and isolate
the process-global SAPI lifetime.
