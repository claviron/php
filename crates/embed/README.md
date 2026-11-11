# claviron-php-embed

In-process PHP execution through a managed ZTS SAPI.

The engine provides request-scoped scripts, named persistent applications,
managed task execution, returned-value evaluation and caller-owned console
commands. Native bridge capabilities are selected explicitly.

Use `claviron_php::embed` through the `claviron-php` facade's `embed` feature,
or depend on this engine directly. A PHP embed toolchain is required;
`PHP_CONFIG` selects it.

The `backend` feature exposes `backend::prepare_script` and
`backend::prepare_application` using `claviron-pipeline`. It selects no Edge
or listener. The facade's backend factory decodes PHP directives and prepares
these owners before request dispatch.

A persistent application runs one exchange per `instances` slot.
`max_queued` counts exchanges that may wait for a slot. Zero disables waiting.
Behind Orbit Link, the runner uses those instances as its execution bound
and configures the only queue with `orbit.link.max_queued`.

Shared configuration and runtime values come from `claviron-php-core`.
PHP-FPM execution belongs to the separate FCGI engine.

The Composer package in the repository's `php/` directory is `claviron/sapi`.
It contains PHP declarations and adapters, not the native engine.

See [ARCHITECTURE.md](ARCHITECTURE.md) for execution ownership and native bridges.
