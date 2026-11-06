---
name: claviron-php-backend
description: Add or revise PHP pipeline backend preparation, route directives, and native execution handoffs for embedded PHP or FastCGI while preserving unread request ownership and shared policy enforcement.
---

# PHP pipeline backends

Begin at [the PHP factory](../../../src/backend.rs), then read only the selected
engine's `src/backend.rs` and adapter. The public registration API is
`claviron_php::backend::register(&SiteRegistry)`; engine providers are registered
separately. Backend preparation uses Pipeline contracts, not Edge host types.

## Cold preparation

Decode PHP declarations and select the concrete engine during boot/reload.
An absent or null `php:` produces no owner. A present directive requires an
explicit runner available in the selected build. Retain existing declaration
precedence and `BackendPlacement`; do not create another request-time engine
selector.

Resolve the effective prefix and files mapping once into `PhpRuntime`. Keep
configuration convenience in the decoder and effective serving paths in the
prepared owner. Reuse the files mapping instead of introducing an independent
PHP path matcher or stripping the request URI.

For persistent applications, bind a named handle during preparation. Request
routes do not redefine the application's boot script, environment or lifecycle.

## Request ownership

Each concrete owner implements `DispatchStep` and `BackendHandler`. Declare
identity and response lifetime through `BackendContract`; Pipeline owns the
common handoff. Script, application and FCGI owners currently request exported
request identity. Embedded responses use `BackendReply::Owner`; FCGI declares
`ExternalEventStream` with its service shutdown token.

Return `Unhandled(original_request)` only before polling/consuming its body.
A response is terminal regardless of its status; a backend 404 is not a signal
to replay the request elsewhere. Do not collect and reconstruct a body merely
to support fallthrough.

Pipeline owns ingress body limits. Consume the admitted body using its effective
policy, without adding the same guard again. Keep request buffering and response
streaming distinct; use the existing [engine adapters](../../../crates/embed/src/routing/adapter.rs)
and [FCGI handoff](../../../crates/fcgi/src/backend.rs) as the contract reference.

Preserve [the execution I/O contract](../../../crates/core/ARCHITECTURE.md#execution_io-opcache_owns_freshness):
a pinned entry script must not acquire redundant per-request existence or
freshness probes. Mixed-directory script validation and explicit `try_files`
have different requirements.

## Verification

Follow [the factory tests](../../../tests/backend.rs) for absent directives,
runner validation and shared mapping. Exercise body ownership with a body that
records polling. A preparation test must not start PHP or PHP-FPM. Changes to
streaming or cancellation need engine-specific checks against the actual owner,
including release on a dropped response, rather than only a mocked success.
