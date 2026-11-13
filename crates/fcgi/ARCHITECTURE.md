# claviron-php-fcgi

## Boundary

This crate owns external PHP-FPM execution over FastCGI. It shares declarations
and prepared runtime values with Embed through `claviron-php-core`, and does
not depend on Embed or the PHP facade. No libphp or native PHP build is needed.

The optional `backend` feature implements the `claviron-pipeline` handoff.
It selects neither Edge nor a listener.

## Sources

- `lib.rs`: lifecycle registration and public engine API.
- `config.rs`: named FPM pools, dial/probe policy and prepared options.
- `service.rs`: configured pool publication and execution ownership.
- `pool.rs`: capacity, idle connections, leases, repair and retirement.
- `connection.rs`: FastCGI connections and exchange lifetime.
- `backend.rs`: native Backend preparation and FPM handoff.
- `proxy.rs`: request mapping, CGI variables, buffered/streamed input and response pumping.
- `balancer.rs`: backend selection.
- `ping.rs`: configured FPM liveness probes.
- `events.rs`: engine-owned pool events.
- `metrics/`: FCGI and FPM metrics with separate ownership.
- `tasks.rs`: optional configured maintenance work.
- `tests/`: configuration, pool, protocol and body ownership contracts.

## Preparation

The application registers `PhpFcgiService`. Congif prepares `fpm.yaml`;
the service creates and publishes its configured pools during lifecycle boot.
A prepared PHP backend resolves the service, retains its selected runtime and
declares identity export and `BackendReply::Relay` through `BackendContract`.
The pipeline applies the prepared response policy to FPM's headers and body.
Small replies can complete in the buffer, overflowing replies continue as
streams, and SSE starts streaming immediately and ends on shutdown.

The PHP facade resolves the site prefix, document mapping and execution runner
at boot/reload. Requests retain their original URI. A pinned entry script
remains relative to the document directory.

## Pools and exchanges

Pools distinguish warm-floor, idle and total live capacity. A connection is
returned to the idle pool only when the exchange has completed safely.
Liveness comes from a successful configured probe or request exchange;
a connected socket alone does not establish FPM health.

Buffered and streaming paths retain the same request/body ownership contract.
The pipeline owns input bounds and outgoing response bounds; FCGI consumes and
forwards the admitted body without installing a second ingress policy.

Dropping a response receiver interrupts setup, reads and channel backpressure.
A pooled connection with unread response bytes becomes unrecoverable before
its lease is released. Cancelling dial admission releases its reservation.
Closing that transport makes no claim that remote PHP execution has stopped.

## Shutdown and reload

Each pool owns its repair cancellation and task tracker. Repair uses the
configured positive backoff/cap and warm floor; disabled repair creates no
background loop. No progress polling or invented retry duration is introduced.

Service cancellation rejects acquisitions, retires entries and drains repair
tasks. Configuration replacement retires only affected pools. Already leased
responses remain owned by their pumps; retired pools never requeue a connection.
A pool cannot cancel its parent application or another pool.

Pinned and mixed-directory script selection follow
[Core's execution IO contract](../core/ARCHITECTURE.md#execution_io-opcache_owns_freshness).
