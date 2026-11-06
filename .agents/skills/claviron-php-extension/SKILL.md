---
name: claviron-php-extension
description: Create or extend a compiled-in PHP Embed extension in Rust, exposing native functions, classes, enums, or Zend observers through PhpEmbedExtension. Applies to the embedded SAPI, not a dynamically loaded PHP module.
---

# Native PHP extensions

Start with the public [extension contract](../../../crates/embed/src/extension.rs).
Import it through `claviron_php::embed::extension` or directly through
`claviron_php_embed::extension`, according to the consuming crate's dependency.
Keep application-specific bindings in that application or its extension crate.

Implement `PhpEmbedExtension` with a stable, unique `name()`. Register one owned
instance using `register_extension(state, Arc::new(extension))` during bootstrap,
before PHP startup freezes the registry. Registration can fail; propagate it.

## Hooks

| Hook | Responsibility |
| --- | --- |
| `configure` | Capture prepared Rust services or immutable configuration during registration. |
| `append_functions` | Add complete function declarations and their argument metadata. |
| `register_classes` | Add classes/enums to the supplied host `ModuleBuilder` for MINIT. |
| `ensure_for_current_thread` | Bind thread-local state after a ZTS context is attached. |
| `observer` | Supply an optional process-wide observer sink. |

For a class, follow the minimal [declaration test](../../../crates/embed/tests/extension.rs):
use `php_class`/`php_impl` and return `module.class::<T>()` from
`register_classes`. Enums use the same builder's `enumeration::<T>()`. Preserve
the host module's lifecycle callbacks; per-request and per-thread hooks do not
register classes.

For a function, follow [the config bridge](../../../crates/embed/src/functions/config.rs).
Push the complete `FunctionBuilder::build()` result into `FunctionEntries`.
Zend borrows both the entry and its argument table; dropping the metadata after
startup leaves invalid pointers. Hand-written C handlers contain panics with
`ext_php_rs::zend::run_handler` and convert failures to PHP exceptions.

## Ownership

The extension is `Send + Sync`, shared across SAPI threads. Keep `Zval`, Zend
objects and borrowed PHP data within their owning native execution/thread
scope. Convert to owned Rust values only when they must cross that boundary;
do not serialize or clone an entire request to obtain one field.

Use [PhpObserver](../../../crates/embed/src/observer.rs) for Zend observations.
Callbacks can run concurrently; request only the required interests, keep
filters cheap, and avoid blocking I/O. The host owns observer registration.

If adding a built-in public PHP API, update the corresponding declarations in
`php/fallback/`. Missing native capabilities must fail clearly; the Composer
package must not silently emulate them. An application-specific extension owns
its own PHP declarations.

Test declaration/validation logic without starting PHP. When native behavior
needs verification, use a separate integration-test process following the
existing `crates/embed/tests/support/native_runtime.rs` harness. Confirm the
function/class is native and its metadata survives repeated ZTS requests;
merely calling an identically named PHP fallback is insufficient.
