# claviron-php

PHP execution for Claviron, with embedded and FastCGI backends.

Add [claviron-php](https://crates.io/crates/claviron-php) to a Claviron host
and select the PHP engine your application uses.

| Package | Responsibility |
| --- | --- |
| `claviron-php` | Public API, engine registration and configured pipeline composition. |
| `claviron-php-core` | Shared PHP configuration, prepared runtime values and header rules. |
| `claviron-php-embed` | In-process PHP SAPI, scripts, persistent applications and native bridges. |
| `claviron-php-fcgi` | External PHP-FPM pools and FastCGI request/response transport. |

Select `embed` or `fcgi` explicitly; the default selects neither engine.
The engines share contracts and do not depend on each other.
The PHP family uses Claviron's individual crates directly.

## Pipeline integration

Add `backend` to prepare PHP route directives through
`claviron_pipeline::SiteRegistry`. It selects no Edge, listener or transport.

```rust,ignore
use claviron_pipeline::SiteRegistry;
use claviron_runtime::{AppState, registry::Result};

fn register_php(state: &AppState, site: &SiteRegistry) -> Result<()> {
    claviron_php::register(state)?;
    claviron_php::backend::register(site)
}
```

Engine registration installs lifecycle providers. Backend registration installs
one cold factory on the supplied site registry. The serving application owns
that registry, its hosts or single site, and its listeners.

The factory reads `php:` at boot/reload, resolves the selected route's document
mapping and prepares concrete engine owners. Dispatch does not decode config or
select a PHP runner again. The admitted request retains its original URI and
body ownership.

```yaml
routes:
  /forms:
    directory: forms
    php:
      runner: fcgi
      fpm: forms
      entry_script: index.php
```

`runner` is required: `fcgi`, `script`, or `application`, as available in the
selected build. Persistent embedded applications select a named definition;
they do not acquire script-mode settings from a route.

## Optional capabilities

`metrics` and `tasks` extend selected engines. `auth`, `cache`,
`log-pipeline`, `sqlite`, `mysql` and `postgres` extend Embed. These features
do not select an engine on their own. Engine-specific APIs remain available
through `claviron_php::embed` and `claviron_php::fcgi`.

## PHP tooling

[Install scripts](scripts/install/README.md) build and manage a PHP ZTS/FPM
installation. They do not run during Cargo builds or engine registration.

The framework-neutral Composer package [claviron/sapi](php/README.md) contains
native API declarations, fail-fast fallbacks and a Monolog handler.
Composer snapshot preparation is configured independently of Cargo versions.

## Development

```sh
scripts/format.sh
scripts/test.sh backend
```

Embed requires a ZTS PHP embed toolchain selected by `PHP_CONFIG`. Keep the
same toolchain environment across builds. Native runtime checks are explicit
integration checks; configuration and adapter tests do not boot PHP.

See [ARCHITECTURE.md](ARCHITECTURE.md) for crate and ownership boundaries.
