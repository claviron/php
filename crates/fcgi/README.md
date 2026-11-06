# claviron-php-fcgi

PHP-FPM execution over FastCGI, with managed backend pools and streaming bodies.

Use `claviron_php::fcgi` through the `claviron-php` facade's `fcgi` feature,
or depend on this engine directly. FCGI does not link libphp or require the
embedded PHP toolchain.

The engine owns FPM configuration, backend selection, leases, CGI variables,
protocol exchanges and its metrics. Shared route configuration and runtime
values come from `claviron-php-core`.

The optional `backend` feature exposes `backend::prepare` against
`claviron-pipeline`. A serving application registers the engine and prepares
a concrete PHP owner at boot/reload. Neither Edge nor a listener is selected.

See [ARCHITECTURE.md](ARCHITECTURE.md) for pool and streaming ownership.
