---
name: claviron-php-application
description: Build or adapt a persistent embedded PHP application using PhpApplication, the receive/respond protocol, and route-bound application handles. Use for boot-once PHP applications, not request-scoped scripts or external PHP-FPM.
---

# Persistent PHP applications

Read [PhpApplication](../../../crates/embed/src/config/application.rs) and the
[runtime builder](../../../crates/embed/src/service.rs). Application instances
boot once on their own PHP threads in each serving process. Request demand
does not create instances; script-mode request pools are separate.

## Preparation

For explicit composition, add a named `PhpApplication` with
`PhpEmbedRuntime::builder().application(...).build().register(state)`.
For an enclosing launch document, use `register_application_from_yaml`;
read its current schema rather than inventing application fields.

Set instance count, queue capacity and execution/retry policies from the
application's declarations. Do not invent timeouts or add a second scheduler.
Keep application boot environment separate from a site's per-request values.

Resolve `PhpApplicationService` during route preparation and bind
`service.application(name)` once. The returned `PhpApplicationHandle` follows
lifecycle generations without a request-time application-name lookup.
`PhpApplicationAdapter` also prepares site environment values when needed.

Prefer the configured PHP backend for site routing. A direct Rust caller must
supply final `HttpPolicy`, an ingress-guarded body, and any required request
identity; an application handle is an execution endpoint, not an ingress layer.

## PHP loop

Use the [native application API](../../../crates/embed/src/functions/application.rs)
and [PHP declarations](../../../php/fallback/functions.php). Bootstrap the
framework before the loop. `claviron_application_receive()` yields an HTTP
request, a runtime event, or `null` on shutdown; handle those cases separately.

Complete each HTTP request with `claviron_application_respond`, or stream via
`claviron_application_stream` / `claviron_application_write` /
`claviron_application_end`. Stop writing when the client is gone. Keep session,
authentication and other request-local framework state scoped to that request;
retained boot state is not a place for the previous user's context.

The engine owns cancellation, instance replacement and drain. Do not replay
accepted requests when an instance fails, or interrupt a different service's
PHP work. No PHP object or resource crosses to another thread.

## Verification

Use pure definition and request-adapter tests for configuration and ownership.
Native lifecycle verification must observe multiple requests on the same
instance, request isolation, and shutdown using an isolated process. Read the
existing `crates/embed/tests/application*.rs` cases before adding a harness;
do not boot a complete serving application to test a parser.
