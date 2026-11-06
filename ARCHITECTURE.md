# PHP workspace

## Dependencies

```text
claviron-php
├── claviron-php-core
├── claviron-php-embed ── claviron-php-core
└── claviron-php-fcgi  ── claviron-php-core
```

The root is the public composition API. Core holds shared declarations and
prepared values; it executes no PHP. Each engine owns its execution, lifecycle,
configuration and metrics. Neither engine depends on the root or its sibling.

Claviron integration uses individual crates. HTTP owns body/policy types;
Pipeline owns backend and site contracts; Runtime owns service lifecycle.
No crate in this workspace requires Edge or Transport. A serving application
registers the engines and attaches the PHP factory to its site registry.

## Sources

- `src/lib.rs`: selected engine exports and lifecycle registration.
- `src/backend.rs`: explicit PHP directive decoding and native owner preparation.
- `crates/core/`: shared `PhpConfig`, `PhpRunner`, `PhpRuntime` and header rules.
- `crates/embed/`: embedded PHP, persistent applications, task execution and bridges.
- `crates/fcgi/`: PHP-FPM pools, FastCGI streaming and metrics.
- `php/`: the framework-neutral Composer SAPI package.
- `scripts/install/`: PHP build, module, CLI and FPM installation tools.
- `scripts/packagist.*`: committed-source Composer snapshot export.

## Preparation and dispatch

`backend::register(&SiteRegistry)` adds a single factory. A missing/null
`php:` prepares no owner. Present declarations require an explicit runner.
The factory resolves document mappings and declared precedence once, then
installs separate native owners for the selected engines. There is no extra
request-time PHP dispatcher.

Backend contracts declare identity export and response lifetime. Pipeline
performs the shared handoff, including outgoing stream shutdown. Engines
consume the admitted body and preserve unread ownership when declining a
request. Body bounds remain with the serving pipeline; PHP receives the
effective policy without adding a second ingress guard.

Native script, application, task, evaluator and console APIs do not require
a site factory or listener. Engine-owned shutdown may affect only work owned
by that execution generation.

## Tests

- `tests/backend.rs`: PHP declaration and prepared mapping contracts.
- `crates/core/tests/`: shared header rules.
- `crates/fcgi/tests/`: pool, configuration, protocol and backend behavior.
- `crates/embed/tests/`: owned values, configuration and native adapters;
  explicit native integration fixtures are separate from pure unit checks.
