//! PHP-facing Claviron cache primitives.
//!
//! `claviron_cache_store()` resolves the configured default or named
//! [`CacheRepository`] once and returns an opaque native handle. The operation
//! functions accept that handle, so PHP framework integrations can keep their
//! selected repository without passing a store name on every call.
//!
//! Tags are separate functions on the same handle:
//! `claviron_cache_supports_tags` answers for the resolved store, and
//! `claviron_cache_put_tagged` and `claviron_cache_invalidate_tags` are refused
//! by a store without tags rather than reduced to untagged calls. Every cache
//! failure is a `Claviron\Runtime\CacheException`; the tag refusals are its
//! subclasses, so PHP tells them apart by class, not by message.

use std::ffi::CString;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use claviron_cache::{
    ArchivedCacheJson, CacheJson, CacheProvider, CacheRepository, CacheValue,
    CacheValueRef, Error as CacheError,
};
use ext_php_rs::args::Arg;
use ext_php_rs::builders::{ClassBuilder, ClassProperty, FunctionBuilder};
use ext_php_rs::class::RegisteredClass;
use ext_php_rs::convert::IntoZval;
use ext_php_rs::exception::PhpException;
use ext_php_rs::flags::{ClassFlags, DataType, PropertyFlags};
use ext_php_rs::php_class;
use ext_php_rs::types::{ArrayKey, ZendClassObject, ZendHashTable, Zval};
use ext_php_rs::zend::{ClassEntry, ExecuteData, ExecutorGlobals, ce};
use tokio::runtime::Handle;
use tracing::{debug, warn};

/// Every cache function failure; the tag refusals below extend it.
const CACHE_EXCEPTION: &str = "Claviron\\Runtime\\CacheException";
/// A tagged call on a store whose `claviron_cache_supports_tags` is false.
const TAGS_UNSUPPORTED_EXCEPTION: &str =
    "Claviron\\Runtime\\CacheTagsUnsupportedException";
/// More tags than the store keeps; carries `count` and `max`.
const TOO_MANY_TAGS_EXCEPTION: &str =
    "Claviron\\Runtime\\CacheTooManyTagsException";
/// An invalidation that names no tag.
const TAGS_EMPTY_EXCEPTION: &str = "Claviron\\Runtime\\CacheTagsEmptyException";

use crate::extension::FunctionEntries;

struct PhpCacheBridge {
    provider: Arc<CacheProvider>,
    runtime: Handle,
}

/// Opaque PHP object holding one already-resolved cache repository.
///
/// PHP cannot construct, extend, clone, or serialize this object. The only
/// creation path is [`store_handler`], analogous to PHP's native handle types.
#[php_class]
#[php(name = "Claviron\\Runtime\\CacheHandle")]
#[php(flags = ClassFlags::from_bits_retain(
    ClassFlags::Final.bits() | ClassFlags::NoDynamicProperties.bits()
))]
pub(crate) struct PhpCacheHandle {
    repository: Arc<CacheRepository>,
}

impl PhpCacheBridge {
    fn repository(
        &self,
        store: Option<&str>,
    ) -> claviron_cache::Result<Arc<CacheRepository>> {
        match store {
            Some("") => Err(claviron_cache::Error::Rejected(
                "cache store name must not be empty".to_string(),
            )),
            Some(name) => self.provider.store(name),
            None => self.provider.default_store(),
        }
    }

    fn block_on<F, T>(
        &self,
        future: F,
    ) -> claviron_cache::Result<T>
    where
        F: Future<Output = claviron_cache::Result<T>>,
    {
        if Handle::try_current().is_ok() {
            return Err(CacheError::Rejected(
                "synchronous PHP cache access must run outside a Tokio runtime thread".to_string()
            ));
        }
        self.runtime.block_on(future)
    }
}

static PHP_CACHE: OnceLock<ArcSwapOption<PhpCacheBridge>> = OnceLock::new();

fn bridge_slot() -> &'static ArcSwapOption<PhpCacheBridge> {
    PHP_CACHE.get_or_init(ArcSwapOption::const_empty)
}

pub(crate) fn configure(
    provider: Option<Arc<CacheProvider>>
) -> claviron_runtime::registry::Result<()> {
    let bridge = provider
        .map(|provider| {
            Handle::try_current()
                .map(|runtime| Arc::new(PhpCacheBridge { provider, runtime }))
                .map_err(|error| {
                    claviron_runtime::registry::Error::msg(format!(
                        "PHP cache runtime handle is unavailable: {error}"
                    ))
                })
        })
        .transpose()?;

    let enabled = bridge.is_some();
    bridge_slot().store(bridge);
    debug!(enabled, "PHP Claviron cache host functions configured");
    Ok(())
}

fn bridge() -> Result<Arc<PhpCacheBridge>, String> {
    bridge_slot()
        .load_full()
        .ok_or_else(|| "Claviron cache is not configured".to_string())
}

pub(crate) fn append_function_entries(
    entries: &mut FunctionEntries
) -> Result<(), String> {
    let functions = [
        FunctionBuilder::new("claviron_cache_store", store_handler)
            .not_required()
            .arg(
                Arg::new("name", DataType::String).allow_null().default("null"),
            )
            .returns(DataType::object(PhpCacheHandle::CLASS_NAME), false, false)
            .build()
            .map_err(|error| format!("claviron_cache_store: {error}"))?,
        FunctionBuilder::new("claviron_cache_get", get_handler)
            .arg(Arg::new(
                "cache",
                DataType::object(PhpCacheHandle::CLASS_NAME),
            ))
            .arg(Arg::new("key", DataType::String))
            .returns(DataType::Mixed, false, true)
            .build()
            .map_err(|error| format!("claviron_cache_get: {error}"))?,
        FunctionBuilder::new("claviron_cache_put", put_handler)
            .arg(Arg::new(
                "cache",
                DataType::object(PhpCacheHandle::CLASS_NAME),
            ))
            .arg(Arg::new("key", DataType::String))
            .arg(Arg::new("value", DataType::Mixed))
            .not_required()
            .arg(
                Arg::new("seconds", DataType::Long)
                    .allow_null()
                    .default("null"),
            )
            .returns(DataType::Bool, false, false)
            .build()
            .map_err(|error| format!("claviron_cache_put: {error}"))?,
        FunctionBuilder::new("claviron_cache_forget", forget_handler)
            .arg(Arg::new(
                "cache",
                DataType::object(PhpCacheHandle::CLASS_NAME),
            ))
            .arg(Arg::new("key", DataType::String))
            .returns(DataType::Bool, false, false)
            .build()
            .map_err(|error| format!("claviron_cache_forget: {error}"))?,
        FunctionBuilder::new("claviron_cache_clear", clear_handler)
            .arg(Arg::new(
                "cache",
                DataType::object(PhpCacheHandle::CLASS_NAME),
            ))
            .returns(DataType::Bool, false, false)
            .build()
            .map_err(|error| format!("claviron_cache_clear: {error}"))?,
        FunctionBuilder::new(
            "claviron_cache_supports_tags",
            supports_tags_handler,
        )
        .arg(Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME)))
        .returns(DataType::Bool, false, false)
        .build()
        .map_err(|error| format!("claviron_cache_supports_tags: {error}"))?,
        FunctionBuilder::new("claviron_cache_put_tagged", put_tagged_handler)
            .arg(Arg::new(
                "cache",
                DataType::object(PhpCacheHandle::CLASS_NAME),
            ))
            .arg(Arg::new("key", DataType::String))
            .arg(Arg::new("value", DataType::Mixed))
            .arg(Arg::new("tags", DataType::Array))
            .not_required()
            .arg(
                Arg::new("seconds", DataType::Long)
                    .allow_null()
                    .default("null"),
            )
            .returns(DataType::Bool, false, false)
            .build()
            .map_err(|error| format!("claviron_cache_put_tagged: {error}"))?,
        FunctionBuilder::new(
            "claviron_cache_invalidate_tags",
            invalidate_tags_handler,
        )
        .arg(Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME)))
        .arg(Arg::new("tags", DataType::Array))
        .returns(DataType::Bool, false, false)
        .build()
        .map_err(|error| format!("claviron_cache_invalidate_tags: {error}"))?,
    ];

    entries.extend(functions);
    debug!(
        "prepared PHP functions for cache: claviron_cache_store/claviron_cache_get/\
         claviron_cache_put/claviron_cache_forget/claviron_cache_clear/\
         claviron_cache_supports_tags/claviron_cache_put_tagged/\
         claviron_cache_invalidate_tags"
    );
    Ok(())
}

/// Declare the cache exception classes. Runs in the host module's MINIT,
/// where internal classes are registered; `RuntimeException` (SPL) is
/// already there.
///
/// These are plain subclasses without a native object, so PHP's own
/// exception constructor records file, line and trace as for any exception.
pub(crate) fn register_exception_classes() -> Result<(), String> {
    if find_class("RuntimeException").is_none() {
        return Err("RuntimeException is not registered".to_string());
    }
    let register = |builder: ClassBuilder, name: &str| {
        builder
            .registration(|_, _| {})
            .register()
            .map_err(|error| format!("{name}: {error}"))
    };
    register(
        ClassBuilder::new(CACHE_EXCEPTION)
            .extends((runtime_exception, "\\RuntimeException")),
        CACHE_EXCEPTION,
    )?;
    let subclass = |name: &str| {
        ClassBuilder::new(name)
            .extends((cache_exception, "\\Claviron\\Runtime\\CacheException"))
            .flags(ClassFlags::Final)
    };
    register(subclass(TAGS_UNSUPPORTED_EXCEPTION), TAGS_UNSUPPORTED_EXCEPTION)?;
    // Set on every throw; an instance PHP constructs itself leaves them null.
    let public = |name: &str| ClassProperty {
        name: name.to_owned(),
        flags: PropertyFlags::Public,
        default: None,
        docs: &[],
        ty: None,
        nullable: true,
        readonly: false,
        default_stub: None,
    };
    register(
        subclass(TOO_MANY_TAGS_EXCEPTION)
            .property(public("count"))
            .property(public("max")),
        TOO_MANY_TAGS_EXCEPTION,
    )?;
    register(subclass(TAGS_EMPTY_EXCEPTION), TAGS_EMPTY_EXCEPTION)?;
    Ok(())
}

fn runtime_exception() -> &'static ClassEntry {
    find_class("RuntimeException")
        .expect("RuntimeException is checked before registration")
}

fn cache_exception() -> &'static ClassEntry {
    find_class(CACHE_EXCEPTION)
        .expect("CacheException is registered before its subclasses")
}

/// An internal class from the class table, without autoloading. Valid in
/// MINIT, before the executor's own table exists, and on request threads.
fn find_class(name: &str) -> Option<&'static ClassEntry> {
    use ext_php_rs::ffi::zend_hash_str_find_ptr_lc;

    // SAFETY: the compiler globals and their class table are owned by PHP
    // for the life of the thread; the lookup lowercases a copy of `name`.
    unsafe {
        let globals = ext_php_rs::ffi::ext_php_rs_compiler_globals();
        let table = globals.as_ref()?.class_table;
        if table.is_null() {
            return None;
        }
        zend_hash_str_find_ptr_lc(table, name.as_ptr().cast(), name.len())
            .cast::<ClassEntry>()
            .as_ref()
    }
}

pub(crate) extern "C" fn store_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        let Some(name) = parse_store_name(ex) else {
            retval.set_null();
            return;
        };
        let Ok(bridge) =
            bridge().inspect_err(|error| throw_cache_exception(error))
        else {
            retval.set_null();
            return;
        };
        let Ok(repository) =
            bridge.repository(name.as_deref()).map_err(throw_cache_error)
        else {
            retval.set_null();
            return;
        };

        if ZendClassObject::new(PhpCacheHandle { repository })
            .set_zval(retval, false)
            .is_err()
        {
            retval.set_null();
            throw_cache_exception(
                "failed to create Claviron\\Runtime\\CacheHandle",
            );
        }
    }));
}

pub(crate) extern "C" fn get_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        let Some((repository, key)) = parse_handle_key(ex) else {
            retval.set_null();
            return;
        };
        let Ok(bridge) =
            bridge().inspect_err(|error| throw_cache_exception(error))
        else {
            return;
        };

        match bridge.block_on(repository.get(&key)) {
            Ok(Some(bytes)) => match CacheValue::view(&bytes) {
                Ok(value) => set_retval_from_cache_value(retval, value),
                Err(error) => throw_cache_exception(error),
            },
            Ok(None) => retval.set_null(),
            Err(error) => throw_cache_error(error),
        }
    }));
}

pub(crate) extern "C" fn put_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        let Ok((repository, key, value, seconds)) =
            parse_put(ex).inspect_err(|error| throw_cache_exception(error))
        else {
            retval.set_bool(false);
            return;
        };
        let Ok(bridge) =
            bridge().inspect_err(|error| throw_cache_exception(error))
        else {
            retval.set_bool(false);
            return;
        };
        let result = if let Some(seconds) = seconds {
            if seconds <= 0 {
                bridge.block_on(repository.forget(&key))
            } else {
                bridge.block_on(repository.put_value(
                    &key,
                    value,
                    Some(Duration::from_secs(seconds as u64)),
                ))
            }
        } else {
            bridge.block_on(repository.put_value(&key, value, None))
        };

        set_bool_result(retval, result);
    }));
}

pub(crate) extern "C" fn forget_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        let Some((repository, key)) = parse_handle_key(ex) else {
            retval.set_bool(false);
            return;
        };
        let Ok(bridge) =
            bridge().inspect_err(|error| throw_cache_exception(error))
        else {
            retval.set_bool(false);
            return;
        };
        set_bool_result(retval, bridge.block_on(repository.forget(&key)));
    }));
}

pub(crate) extern "C" fn clear_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        let Some(repository) = parse_handle(ex) else {
            retval.set_bool(false);
            return;
        };
        let Ok(bridge) =
            bridge().inspect_err(|error| throw_cache_exception(error))
        else {
            retval.set_bool(false);
            return;
        };
        set_bool_result(retval, bridge.block_on(repository.clear()));
    }));
}

pub(crate) extern "C" fn supports_tags_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        retval.set_bool(false);
        let mut cache =
            Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME));
        let parsed = ex
            .parser()
            .arg(&mut cache)
            .parse()
            .map_err(|error| Failure::Bridge(error.to_string()))
            .and_then(|()| handle_arg(&mut cache, "supports_tags"));
        match parsed {
            Ok(repository) => retval.set_bool(repository.supports_tags()),
            Err(failure) => failure.throw(),
        }
    }));
}

pub(crate) extern "C" fn put_tagged_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        retval.set_bool(false);
        match put_tagged(ex) {
            Ok(()) => retval.set_bool(true),
            Err(failure) => failure.throw(),
        }
    }));
}

pub(crate) extern "C" fn invalidate_tags_handler(
    ex: &mut ExecuteData,
    retval: &mut Zval,
) {
    ext_php_rs::zend::run_handler(std::panic::AssertUnwindSafe(|| {
        retval.set_bool(false);
        match invalidate_tags(ex) {
            Ok(()) => retval.set_bool(true),
            Err(failure) => failure.throw(),
        }
    }));
}

/// The arguments borrow the caller's key and tag strings for the call; only
/// the value is converted, as `claviron_cache_put` converts it.
fn put_tagged(ex: &mut ExecuteData) -> Result<(), Failure> {
    const FUNCTION: &str = "put_tagged";
    let mut cache =
        Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME));
    let mut key = Arg::new("key", DataType::String);
    let mut value = Arg::new("value", DataType::Mixed);
    let mut tags = Arg::new("tags", DataType::Array);
    let mut seconds = Arg::new("seconds", DataType::Long).allow_null();
    ex.parser()
        .arg(&mut cache)
        .arg(&mut key)
        .arg(&mut value)
        .arg(&mut tags)
        .not_required()
        .arg(&mut seconds)
        .parse()
        .map_err(|error| Failure::Bridge(error.to_string()))?;

    let repository = handle_arg(&mut cache, FUNCTION)?;
    let key = key_arg(&mut key, FUNCTION)?;
    let value =
        value.zval().and_then(|value| cache_value_from_zval(value)).ok_or_else(
            || {
                Failure::Bridge(
                    "unsupported cache value; expected null, bool, int, float, string, bytes, or array"
                        .to_string(),
                )
            },
        )?;
    let tags = tags_arg(&mut tags, FUNCTION, 4)?;
    let seconds = optional_i64(seconds).ok_or_else(|| {
        Failure::Bridge("cache TTL must be an integer".to_string())
    })?;
    let bridge = bridge().map_err(Failure::Bridge)?;

    match seconds {
        // As claviron_cache_put: a non-positive TTL forgets the key. The
        // store must still support tags, so the refusal stays at this call.
        Some(seconds) if seconds <= 0 => {
            if !repository.supports_tags() {
                return Err(Failure::Cache(CacheError::TagsUnsupported {
                    store: repository.store_name().into(),
                    driver: repository.driver_name().into(),
                }));
            }
            bridge.block_on(repository.forget(key))
        }
        seconds => bridge.block_on(repository.put_value_tagged(
            key,
            value,
            seconds.map(|seconds| Duration::from_secs(seconds as u64)),
            &tags,
        )),
    }
    .map_err(Failure::Cache)
}

fn invalidate_tags(ex: &mut ExecuteData) -> Result<(), Failure> {
    const FUNCTION: &str = "invalidate_tags";
    let mut cache =
        Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME));
    let mut tags = Arg::new("tags", DataType::Array);
    ex.parser()
        .arg(&mut cache)
        .arg(&mut tags)
        .parse()
        .map_err(|error| Failure::Bridge(error.to_string()))?;

    let repository = handle_arg(&mut cache, FUNCTION)?;
    let tags = tags_arg(&mut tags, FUNCTION, 2)?;
    let bridge = bridge().map_err(Failure::Bridge)?;
    bridge.block_on(repository.invalidate_tags(&tags)).map_err(Failure::Cache)
}

/// Why a tagged call failed, thrown as the PHP class that names it.
enum Failure {
    /// An argument of the wrong type: `TypeError`.
    Type(String),
    /// A string argument that is not UTF-8: `ValueError`.
    Value(String),
    /// The bridge or a conversion failed: `CacheException`.
    Bridge(String),
    /// The store's own answer, typed by [`throw_cache_error`].
    Cache(CacheError),
}

impl Failure {
    fn throw(self) {
        match self {
            Self::Type(message) => {
                PhpException::new(message, 0, ce::type_error()).throw();
            }
            Self::Value(message) => {
                PhpException::new(message, 0, ce::value_error()).throw();
            }
            Self::Bridge(message) => throw_cache_exception(message),
            Self::Cache(error) => throw_cache_error(error),
        }
    }
}

fn handle_arg(
    arg: &mut Arg<'_>,
    function: &str,
) -> Result<Arc<CacheRepository>, Failure> {
    repository_from_arg(arg).ok_or_else(|| {
        Failure::Type(format!(
            "claviron_cache_{function}(): Argument #1 ($cache) must be of type {}, {} given",
            PhpCacheHandle::CLASS_NAME,
            php_type(arg.zval().map(|value| &**value)),
        ))
    })
}

fn key_arg<'a>(
    arg: &'a mut Arg<'_>,
    function: &str,
) -> Result<&'a str, Failure> {
    let value = arg.zval().map(|value| &**value);
    let Some(key) = value.and_then(Zval::zend_str) else {
        return Err(Failure::Type(format!(
            "claviron_cache_{function}(): Argument #2 ($key) must be of type string, {} given",
            php_type(value),
        )));
    };
    key.as_str().map_err(|_| {
        Failure::Value(format!(
            "claviron_cache_{function}(): Argument #2 ($key) must be valid UTF-8"
        ))
    })
}

/// The tag strings, borrowed from the caller's array. Keys are ignored;
/// every value must be a UTF-8 string, and nothing is coerced.
fn tags_arg<'a>(
    arg: &'a mut Arg<'_>,
    function: &str,
    position: u8,
) -> Result<Vec<&'a str>, Failure> {
    let value = arg.zval().map(|value| &**value);
    let Some(array) = value.and_then(Zval::array) else {
        return Err(Failure::Type(format!(
            "claviron_cache_{function}(): Argument #{position} ($tags) must be of type array, {} given",
            php_type(value),
        )));
    };
    let mut tags = Vec::with_capacity(array.len());
    for (key, tag) in array {
        let Some(text) = tag.zend_str() else {
            return Err(Failure::Type(format!(
                "claviron_cache_{function}(): Argument #{position} ($tags) must contain only strings, {} given at key {key}",
                php_type(Some(tag)),
            )));
        };
        tags.push(text.as_str().map_err(|_| {
            Failure::Value(format!(
                "claviron_cache_{function}(): Argument #{position} ($tags) must contain only valid UTF-8 strings, invalid at key {key}"
            ))
        })?);
    }
    Ok(tags)
}

fn php_type(value: Option<&Zval>) -> &'static str {
    let Some(value) = value.map(Zval::dereference) else {
        return "nothing";
    };
    if value.is_null() {
        "null"
    } else if value.is_bool() {
        "bool"
    } else if value.is_long() {
        "int"
    } else if value.is_double() {
        "float"
    } else if value.is_string() {
        "string"
    } else if value.is_array() {
        "array"
    } else if value.is_object() {
        "object"
    } else {
        "another type"
    }
}

fn set_bool_result(
    retval: &mut Zval,
    result: claviron_cache::Result<()>,
) {
    match result {
        Ok(()) => retval.set_bool(true),
        Err(error) => {
            retval.set_bool(false);
            throw_cache_error(error);
        }
    }
}

fn throw_cache_exception(error: impl std::fmt::Display) {
    throw_cache_class(
        CACHE_EXCEPTION,
        &format!("Claviron cache operation failed: {error}"),
        &[],
    );
}

/// Throw the class that names a store's refusal, or `CacheException`.
fn throw_cache_error(error: CacheError) {
    let message = format!("Claviron cache operation failed: {error}");
    match error {
        CacheError::TagsUnsupported { .. } => {
            throw_cache_class(TAGS_UNSUPPORTED_EXCEPTION, &message, &[]);
        }
        CacheError::TooManyTags { count, max } => {
            match (i64::try_from(count), i64::try_from(max)) {
                (Ok(count), Ok(max)) => throw_cache_class(
                    TOO_MANY_TAGS_EXCEPTION,
                    &message,
                    &[("count", count), ("max", max)],
                ),
                // Not representable as a PHP int: the numbers stay in the
                // message rather than being clamped.
                _ => throw_cache_class(CACHE_EXCEPTION, &message, &[]),
            }
        }
        CacheError::TagsEmpty => {
            throw_cache_class(TAGS_EMPTY_EXCEPTION, &message, &[]);
        }
        _ => throw_cache_class(CACHE_EXCEPTION, &message, &[]),
    }
}

/// Throw `class` through PHP's own exception construction, so file, line
/// and trace are recorded, then set its public `properties`.
fn throw_cache_class(
    class: &str,
    message: &str,
    properties: &[(&str, i64)],
) {
    if ExecutorGlobals::has_exception() {
        return;
    }
    let Some(entry) = find_class(class) else {
        PhpException::from_message(format!(
            "{class} is not registered; {message}"
        ))
        .throw();
        return;
    };
    // Interior NULs are escaped, so the conversion cannot fail.
    let Ok(text) = CString::new(message.replace('\0', "\\0")) else {
        PhpException::from_message(message.replace('\0', "\\0")).throw();
        return;
    };
    // SAFETY: `entry` is a registered, concrete exception class and `text`
    // is NUL-terminated for the call. The returned object is the pending
    // exception, owned by the executor.
    let object = unsafe {
        ext_php_rs::ffi::zend_throw_exception(
            std::ptr::from_ref(entry).cast_mut(),
            text.as_ptr(),
            0,
        )
        .as_mut()
    };
    let Some(object) = object else {
        return;
    };
    for &(name, value) in properties {
        if let Err(error) = object.set_property(name, value) {
            warn!(class, property = name, %error, "cache exception property not set");
        }
    }
}

fn parse_handle_key(
    ex: &mut ExecuteData
) -> Option<(Arc<CacheRepository>, String)> {
    let mut cache =
        Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME));
    let mut key = Arg::new("key", DataType::String);
    ex.parser().arg(&mut cache).arg(&mut key).parse().ok()?;
    Some((repository_from_arg(&mut cache)?, key.consume::<String>().ok()?))
}

fn parse_handle(ex: &mut ExecuteData) -> Option<Arc<CacheRepository>> {
    let mut cache =
        Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME));
    ex.parser().arg(&mut cache).parse().ok()?;
    repository_from_arg(&mut cache)
}

fn repository_from_arg(arg: &mut Arg<'_>) -> Option<Arc<CacheRepository>> {
    let object = arg.zval()?.object()?;
    let handle = ZendClassObject::<PhpCacheHandle>::from_zend_obj(object)?;
    Some(handle.repository.clone())
}

fn parse_store_name(ex: &mut ExecuteData) -> Option<Option<String>> {
    let mut name = Arg::new("name", DataType::String).allow_null();
    ex.parser().not_required().arg(&mut name).parse().ok()?;
    optional_string(name)
}

fn parse_put(
    ex: &mut ExecuteData
) -> Result<(Arc<CacheRepository>, String, CacheValue, Option<i64>), String> {
    let mut cache =
        Arg::new("cache", DataType::object(PhpCacheHandle::CLASS_NAME));
    let mut key = Arg::new("key", DataType::String);
    let mut value = Arg::new("value", DataType::Mixed);
    let mut seconds = Arg::new("seconds", DataType::Long).allow_null();
    ex.parser()
        .arg(&mut cache)
        .arg(&mut key)
        .arg(&mut value)
        .not_required()
        .arg(&mut seconds)
        .parse()
        .map_err(|error| error.to_string())?;

    let repository = repository_from_arg(&mut cache).ok_or_else(|| {
        "cache handle must be a Claviron\\Runtime\\CacheHandle".to_string()
    })?;
    let key = key
        .consume::<String>()
        .map_err(|_| "cache key must be a string".to_string())?;
    let value = value.zval().and_then(|value| cache_value_from_zval(value)).ok_or_else(|| {
        "unsupported cache value; expected null, bool, int, float, string, bytes, or array"
            .to_string()
    })?;
    let seconds = optional_i64(seconds)
        .ok_or_else(|| "cache TTL must be an integer".to_string())?;
    Ok((repository, key, value, seconds))
}

fn optional_string(mut arg: Arg<'_>) -> Option<Option<String>> {
    match arg.zval() {
        None => Some(None),
        Some(value) if value.is_null() => Some(None),
        Some(_) => arg.consume::<String>().ok().map(Some),
    }
}

fn optional_i64(mut arg: Arg<'_>) -> Option<Option<i64>> {
    match arg.zval() {
        None => Some(None),
        Some(value) if value.is_null() => Some(None),
        Some(_) => arg.consume::<i64>().ok().map(Some),
    }
}

fn cache_value_from_zval(zval: &Zval) -> Option<CacheValue> {
    if zval.is_null() {
        return Some(CacheValue::Null);
    }
    if let Some(value) = zval.bool() {
        return Some(CacheValue::Bool(value));
    }
    if let Some(value) = zval.long() {
        return if value >= 0 {
            Some(CacheValue::U64(value as u64))
        } else {
            Some(CacheValue::I64(value))
        };
    }
    if let Some(value) = zval.double() {
        return Some(CacheValue::F64(value));
    }
    if let Some(value) = zval.string() {
        return Some(CacheValue::String(value));
    }
    if zval.is_array() {
        return cache_json_from_zval(zval).map(CacheValue::Json);
    }
    zval.binary::<u8>().map(CacheValue::Bytes)
}

fn set_retval_from_cache_value(
    retval: &mut Zval,
    value: CacheValueRef<'_>,
) {
    match value {
        CacheValueRef::Null => retval.set_null(),
        CacheValueRef::Bool(value) => retval.set_bool(value),
        CacheValueRef::I64(value) => retval.set_long(value),
        CacheValueRef::U64(value) => set_u64(retval, value),
        CacheValueRef::F64(value) => retval.set_double(value),
        CacheValueRef::String(value) => {
            if retval.set_string(value, false).is_err() {
                retval.set_null();
            }
        }
        CacheValueRef::Bytes(value) => {
            retval
                .set_zend_string(ext_php_rs::types::ZendStr::new(value, false));
        }
        CacheValueRef::Json(value) => {
            if set_zval_from_archived_cache_json(retval, value).is_err() {
                retval.set_null();
            }
        }
        CacheValueRef::LegacyJson(value) => {
            if set_zval_from_cache_json(retval, &value).is_err() {
                retval.set_null();
            }
        }
    }
}

fn set_zval_from_archived_cache_json(
    retval: &mut Zval,
    value: &ArchivedCacheJson,
) -> Result<(), ext_php_rs::error::Error> {
    match value {
        ArchivedCacheJson::Null => retval.set_null(),
        ArchivedCacheJson::Bool(value) => retval.set_bool(*value),
        ArchivedCacheJson::I64(value) => retval.set_long(value.to_native()),
        ArchivedCacheJson::U64(value) => {
            let value = value.to_native();
            if let Ok(value) = i64::try_from(value) {
                retval.set_long(value);
            } else {
                retval.set_double(value as f64);
            }
        }
        ArchivedCacheJson::F64(value) => retval.set_double(value.to_native()),
        ArchivedCacheJson::String(value) => {
            retval.set_string(value.as_str(), false)?;
        }
        ArchivedCacheJson::Array(values) => {
            let mut array = ZendHashTable::with_capacity(
                values.len().try_into().unwrap_or(u32::MAX),
            );
            for value in values.iter() {
                let mut zval = Zval::new();
                set_zval_from_archived_cache_json(&mut zval, value)?;
                array.push(zval)?;
            }
            retval.set_hashtable(array);
        }
        ArchivedCacheJson::Object(values) => {
            let mut array = ZendHashTable::with_capacity(
                values.len().try_into().unwrap_or(u32::MAX),
            );
            for pair in values.iter() {
                let mut zval = Zval::new();
                set_zval_from_archived_cache_json(&mut zval, &pair.1)?;
                array.insert(ArrayKey::Str(pair.0.as_str()), zval)?;
            }
            retval.set_hashtable(array);
        }
    }
    Ok(())
}

fn cache_json_from_zval(zval: &Zval) -> Option<CacheJson> {
    if zval.is_null() {
        return Some(CacheJson::Null);
    }
    if let Some(value) = zval.bool() {
        return Some(CacheJson::Bool(value));
    }
    if let Some(value) = zval.long() {
        return if value >= 0 {
            Some(CacheJson::U64(value as u64))
        } else {
            Some(CacheJson::I64(value))
        };
    }
    if let Some(value) = zval.double() {
        return value.is_finite().then_some(CacheJson::F64(value));
    }
    if let Some(value) = zval.string() {
        return Some(CacheJson::String(value));
    }

    let array = zval.array()?;
    if array.has_sequential_keys() {
        let values = array
            .values()
            .map(cache_json_from_zval)
            .collect::<Option<Vec<_>>>()?;
        return Some(CacheJson::Array(values));
    }

    let mut object = Vec::with_capacity(array.len());
    for (key, value) in array {
        object.push((key.to_string(), cache_json_from_zval(value)?));
    }
    Some(CacheJson::Object(object))
}

fn set_zval_from_cache_json(
    retval: &mut Zval,
    value: &CacheJson,
) -> Result<(), ext_php_rs::error::Error> {
    match value {
        CacheJson::Null => {
            retval.set_null();
            Ok(())
        }
        CacheJson::Bool(value) => {
            retval.set_bool(*value);
            Ok(())
        }
        CacheJson::I64(value) => {
            retval.set_long(*value);
            Ok(())
        }
        CacheJson::U64(value) => {
            if let Ok(value) = i64::try_from(*value) {
                retval.set_long(value);
            } else {
                retval.set_double(*value as f64);
            }
            Ok(())
        }
        CacheJson::F64(value) => {
            retval.set_double(*value);
            Ok(())
        }
        CacheJson::String(value) => retval.set_string(value, false).map(|_| ()),
        CacheJson::Array(values) => {
            let mut array = ZendHashTable::with_capacity(
                values.len().try_into().unwrap_or(u32::MAX),
            );
            for value in values {
                let mut zval = Zval::new();
                set_zval_from_cache_json(&mut zval, value)?;
                array.push(zval)?;
            }
            retval.set_hashtable(array);
            Ok(())
        }
        CacheJson::Object(values) => {
            let mut array = ZendHashTable::with_capacity(
                values.len().try_into().unwrap_or(u32::MAX),
            );
            for (key, value) in values {
                let mut zval = Zval::new();
                set_zval_from_cache_json(&mut zval, value)?;
                array.insert(ArrayKey::String(key.clone()), zval)?;
            }
            retval.set_hashtable(array);
            Ok(())
        }
    }
}

fn set_u64(
    retval: &mut Zval,
    value: u64,
) {
    if let Ok(value) = i64::try_from(value) {
        retval.set_long(value);
    } else {
        let value = value.to_string();
        if retval.set_string(&value, false).is_err() {
            retval.set_null();
        }
    }
}
