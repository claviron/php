<?php

declare(strict_types=1);

namespace Claviron\Runtime;

/**
 * Immutable event delivered by the Claviron embed runtime.
 *
 * @property-read null|bool|int|float|string|array $payload
 */
if (!class_exists(Event::class, false)) {
    final readonly class Event
    {
        private function __construct(
            public string $id,
            public string $topic,
            public mixed $payload,
        ) {}
    }
}

/**
 * Immutable invocation delivered by orbit_invoke_receive().
 *
 * @property-read null|bool|int|float|string|array $payload
 */
if (!class_exists(Invocation::class, false)) {
    final readonly class Invocation
    {
        private function __construct(
            public string $id,
            public string $operation,
            public mixed $payload,
            public int $submitted_at,
        ) {}
    }
}

/**
 * A typed value that lives in the fleet's shared memory and travels by id.
 *
 * `$n = Orbital::int(0); $n++;` updates the same value from every process
 * that holds a handle; `serialize($n)` carries only the id, and the handle
 * that comes back reads whatever the value is then. `+` and `-` on an int or
 * float Orbital are atomic and return the same handle; an int wraps on
 * overflow. Comparisons and casts read the current value.
 *
 * Creating an Orbital is a declaration: it takes a slot of the fleet's table
 * for the life of the fleet and nothing gives it back. Declare the values a
 * program shares once and pass their ids; never create one per request.
 */
if (!class_exists(Orbital::class, false)) {
    final class Orbital
    {
        private function __construct() {}

        public static function int(int $value = 0): self { throw new \RuntimeException('requires the Claviron runtime'); }
        public static function float(float $value = 0.0): self { throw new \RuntimeException('requires the Claviron runtime'); }
        public static function bool(bool $value = false): self { throw new \RuntimeException('requires the Claviron runtime'); }
        /** Reattach to a value by the id another handle printed or serialized. */
        public static function open(string $id): self { throw new \RuntimeException('requires the Claviron runtime'); }

        /** `<kind>:<slot>:<epoch>`, such as `i64:12:3`; valid until the fleet's table is reset. */
        public function id(): string { throw new \RuntimeException('requires the Claviron runtime'); }
        public function get(): int|float|bool { throw new \RuntimeException('requires the Claviron runtime'); }
        public function set(int|float|bool $value): void { throw new \RuntimeException('requires the Claviron runtime'); }
        /** Atomic add for an int or float Orbital; returns the value after. */
        public function add(int|float $by): int|float { throw new \RuntimeException('requires the Claviron runtime'); }
        /**
         * Block until the value is no longer $current, whichever process writes it, and return
         * what it is then; at once when it already differs. Many writes while blocked are one
         * return, of the latest value. Null when this execution was cancelled instead, which
         * claviron_is_cancelled() then reports.
         */
        public function wait(int|float|bool $current): int|float|bool|null { throw new \RuntimeException('requires the Claviron runtime'); }
    }
}

if (!class_exists(CacheHandle::class, false)) {
    /** Opaque native cache handle; only claviron_cache_store() creates one. */
    final class CacheHandle
    {
        private function __construct() {}
        private function __clone() {}

        public function __serialize(): array
        {
            throw new \RuntimeException('CacheHandle requires the Claviron embed runtime and native cache support.');
        }

        public function __unserialize(array $data): void
        {
            throw new \RuntimeException('CacheHandle requires the Claviron embed runtime and native cache support.');
        }
    }
}

if (!class_exists(CacheException::class, false)) {
    /** Every failure of a claviron_cache_* function; the tag refusals extend it. */
    class CacheException extends \RuntimeException {}
}

if (!class_exists(CacheTagsUnsupportedException::class, false)) {
    /** A tagged call on a store without tags. Its untagged operations keep working. */
    final class CacheTagsUnsupportedException extends CacheException {}
}

if (!class_exists(CacheTooManyTagsException::class, false)) {
    /** More tags than the store keeps on one entry or takes in one invalidation. */
    final class CacheTooManyTagsException extends CacheException
    {
        /** @var int|null How many tags the store counted; the runtime sets it when it throws. */
        public $count;

        /** @var int|null How many it keeps for this call; a long key can leave room for fewer. */
        public $max;
    }
}

if (!class_exists(CacheTagsEmptyException::class, false)) {
    /** An invalidation that names no tag. */
    final class CacheTagsEmptyException extends CacheException {}
}

if (!class_exists(PostgresConnection::class, false)) {
    /** Opaque named-pool handle; only claviron_postgres_connection() creates one. */
    final class PostgresConnection
    {
        private function __construct() {}
        private function __clone() {}
    }
}

if (!class_exists(PostgresCursor::class, false)) {
    /** Opaque server-side cursor on the request's connection; only claviron_postgres_cursor() creates one. */
    final class PostgresCursor
    {
        private function __construct() {}
        private function __clone() {}
    }
}

if (!class_exists(SqliteConnection::class, false)) {
    /** Opaque named-pool handle; only claviron_sqlite_connection() creates one. */
    final class SqliteConnection
    {
        private function __construct() {}
        private function __clone() {}
    }
}

if (!class_exists(MySqlConnection::class, false)) {
    /** Opaque named-pool handle; only claviron_mysql_connection() creates one. */
    final class MySqlConnection
    {
        private function __construct() {}
        private function __clone() {}
    }
}
