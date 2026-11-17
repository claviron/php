<?php

declare(strict_types=1);

use Claviron\Runtime\CacheException;
use Claviron\Runtime\CacheTagsEmptyException;
use Claviron\Runtime\CacheTagsUnsupportedException;
use Claviron\Runtime\CacheTooManyTagsException;

set_exception_handler(static function (Throwable $error): void {
    http_response_code(500);
    echo $error;
});

function require_same(mixed $expected, mixed $actual): void
{
    if ($expected !== $actual) {
        throw new RuntimeException(var_export([$expected, $actual], true));
    }
}

/** The exact class, not only a parent, so each refusal is told apart. */
function require_throws(string $class, callable $call): Throwable
{
    try {
        $call();
    } catch (Throwable $error) {
        require_same($class, $error::class);
        return $error;
    }
    throw new RuntimeException("expected {$class}");
}

// The harness runs this file more than once against the same stores.
$tagged = claviron_cache_store('tagged');
$plain = claviron_cache_store();
claviron_cache_forget($plain, 'plain:key');

foreach ([CacheTagsUnsupportedException::class, CacheTooManyTagsException::class, CacheTagsEmptyException::class] as $class) {
    require_same(true, is_subclass_of($class, CacheException::class));
}
require_same(true, is_subclass_of(CacheException::class, RuntimeException::class));

// A store with tags: invalidating one tag removes its entries, not the rest.
require_same(true, claviron_cache_supports_tags($tagged));
require_same(true, claviron_cache_put_tagged($tagged, 'user:1', ['name' => 'Ada'], ['users', 'team:7']));
require_same(true, claviron_cache_put_tagged($tagged, 'user:2', 'Grace', ['users']));
require_same(true, claviron_cache_put($tagged, 'config', 'untagged'));
require_same(['name' => 'Ada'], claviron_cache_get($tagged, 'user:1'));
require_same(true, claviron_cache_invalidate_tags($tagged, ['team:7']));
require_same(null, claviron_cache_get($tagged, 'user:1'));
require_same('Grace', claviron_cache_get($tagged, 'user:2'));
require_same('untagged', claviron_cache_get($tagged, 'config'));

// A store without tags refuses at the tagged call and keeps serving the rest.
require_same(false, claviron_cache_supports_tags($plain));
require_throws(CacheTagsUnsupportedException::class, fn () => claviron_cache_put_tagged($plain, 'plain:key', 'value', ['users']));
require_throws(CacheTagsUnsupportedException::class, fn () => claviron_cache_invalidate_tags($plain, ['users']));
require_same(null, claviron_cache_get($plain, 'plain:key'));
require_same(true, claviron_cache_put($plain, 'plain:key', 'value'));
require_same('value', claviron_cache_get($plain, 'plain:key'));

// Bounds carry their numbers; nothing is written.
$error = require_throws(CacheTooManyTagsException::class, fn () => claviron_cache_put_tagged($tagged, 'many', 1, ['a', 'b', 'c']));
require_same([3, 2], [$error->count, $error->max]);
require_same(null, claviron_cache_get($tagged, 'many'));
require_throws(CacheTagsEmptyException::class, fn () => claviron_cache_invalidate_tags($tagged, []));

// A tag that is not a string is refused, not coerced.
require_throws(TypeError::class, fn () => claviron_cache_put_tagged($tagged, 'coerced', 1, ['users', 7]));
require_throws(TypeError::class, fn () => claviron_cache_invalidate_tags($tagged, [null]));
require_same(null, claviron_cache_get($tagged, 'coerced'));

header('X-Fixture: ok');
echo 'ok';
