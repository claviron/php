---
name: claviron-php
description: Integrate embedded PHP or PHP-FPM into a Claviron application, select Cargo capabilities, and register engine providers while preserving the PHP workspace dependency boundaries.
---

# PHP integration

Read the [workspace architecture](../../../ARCHITECTURE.md) and the consuming
application's manifest before choosing an engine or registration path.

## Composition

| Need | API |
| --- | --- |
| Embedded PHP | `claviron_php::embed` or `claviron_php_embed` |
| External PHP-FPM | `claviron_php::fcgi` or `claviron_php_fcgi` |
| Configured PHP pipeline owners | `claviron_php::backend::register(&SiteRegistry)` |
| Shared declarations and prepared values | `claviron_php_core` |

`claviron_php::register(state)` registers selected engine providers. It does
not create a listener or install a backend factory. A native caller can use an
engine without site routing; a serving application supplies its own registry
and transport.

Read the current [feature declarations](../../../Cargo.toml). Defaults select
neither engine. `backend` and capability features extend the selected engines;
they must not silently select another engine. Keep dependency features at the
consuming crate, rather than in workspace dependency declarations.

Engine crates depend on Core, never on the PHP facade or on each other. Shared
configuration belongs in Core; execution and lifecycle belong to an engine;
configured composition belongs in the facade. Use individual Claviron crates
for HTTP, Pipeline and Runtime contracts. This workspace does not own Edge,
listeners or transport implementations.

## Registration

For Embed, choose either library-owned `php.yaml` loading or explicit
`PhpEmbedRuntime::builder()` composition. Reuse the registered runtime; do not
introduce a second process SAPI to add a program or application. Provider
registration prepares lifecycle ownership; it does not mean PHP has booted.

For implementation details, read only the relevant engine's `src/lib.rs` and
`ARCHITECTURE.md`. Check the consuming graph with `cargo tree` when changing
feature forwarding, especially that an FCGI-only application excludes Embed.

Use the repository's format/test script help for authorized Rust verification.
Keep pure configuration checks separate from native PHP integration checks;
report which actually ran. Use the configured ZTS toolchain without rewriting
the caller's toolchain environment.
