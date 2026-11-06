# claviron-php-core

Shared PHP configuration and prepared runtime contracts for Embed and FastCGI.
This crate executes no PHP and depends on neither engine.

Applications normally use the workspace's `claviron-php` facade. Engine crates
use Core directly so sharing a runtime value never creates a facade dependency
cycle.

See [ARCHITECTURE.md](ARCHITECTURE.md) for policy and ownership boundaries.
