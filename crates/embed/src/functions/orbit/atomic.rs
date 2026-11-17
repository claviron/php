//! `Claviron\Runtime\Orbital`: a fleet-shared atomic value as a PHP value.
//!
//! The object is a handle; the value is 64 bits in the fleet's Orbitals
//! table, and every process that opens the same id reads and updates the
//! same value. Operators come from the class's own Zend object handlers,
//! installed once at module startup: `$n++` and `$n += 5` apply atomically
//! and hand back the same handle; `$n > 10`, `(int) $n`, `echo $n` read it.
//! `serialize()` carries only the id, so an Orbital crosses any boundary a
//! PHP value crosses (a cache, an invocation, a session) by reference.
//!
//! Nothing releases a value. Creating one takes a slot of the fleet's table
//! for as long as that table lives, so a program declares its Orbitals once
//! and shares their ids; it does not create one per request.

use std::cmp::Ordering;
use std::ffi::c_int;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as MemoryOrdering};

use arc_swap::ArcSwapOption;
use claviron_orbit::atomic::{
    Kind, Orbital, OrbitalId, Orbitals, OrbitalsProvider,
};
use claviron_runtime::registry;
use ext_php_rs::boxed::ZBox;
use ext_php_rs::class::RegisteredClass;
use ext_php_rs::exception::PhpException;
use ext_php_rs::ffi::{
    ZEND_RESULT_CODE_FAILURE, ZEND_RESULT_CODE_SUCCESS, zend_object, zval,
};
use ext_php_rs::flags::ClassFlags;
use ext_php_rs::types::{ZendClassObject, ZendHashTable, ZendObject, Zval};
use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::{php_class, php_impl};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::engine::{ClavironContext, PhpExecution};
use crate::sapi::ServerContext;

// Zend VM opcodes `do_operation` is called with; from zend_vm_opcodes.h.
const ZEND_ADD: u8 = 1;
const ZEND_SUB: u8 = 2;

// Cast targets, from zend_types.h.
const IS_LONG: c_int = 4;
const IS_DOUBLE: c_int = 5;
const IS_STRING: c_int = 6;
const IS_BOOL: c_int = 18;

/// The fleet's table, and the runtime that carries a cancellation to a
/// PHP thread parked in `wait()`.
struct Bridge {
    orbitals: Arc<Orbitals>,
    runtime: Handle,
}

static BRIDGE: ArcSwapOption<Bridge> = ArcSwapOption::const_empty();

pub(crate) fn configure(
    provider: Arc<OrbitalsProvider>
) -> registry::Result<()> {
    let bridge = provider
        .handle()
        .map(|orbitals| {
            Handle::try_current()
                .map(|runtime| Arc::new(Bridge { orbitals, runtime }))
                .map_err(|error| {
                    registry::Error::msg(format!(
                        "PHP Orbital runtime handle is unavailable: {error}"
                    ))
                })
        })
        .transpose()?;
    let enabled = bridge.is_some();
    BRIDGE.store(bridge);
    debug!(enabled, "PHP Orbitals configured");
    Ok(())
}

fn bridge() -> Result<Arc<Bridge>, PhpException> {
    BRIDGE.load_full().ok_or_else(|| {
        PhpException::from_message(
            "Orbitals are not available in this process".into(),
        )
    })
}

fn failure(error: impl std::fmt::Display) -> PhpException {
    PhpException::from_message(format!("Orbital: {error}"))
}

/// What ends a wait from outside: the token `claviron_is_cancelled()` reads
/// for a program, and the request pool's drain for a request.
fn cancellation() -> CancellationToken {
    let globals = SapiGlobals::get();
    if !globals.server_context.is_null()
        && let PhpExecution::Program(program) =
            &ClavironContext::from_server_context(globals.server_context)
                .execution
    {
        return program.cancellation.clone();
    }
    crate::engine::sapi_shutdown_signal()
}

#[derive(Clone)]
enum Value {
    Int(Orbital<i64>),
    Float(Orbital<f64>),
    Bool(Orbital<bool>),
}

impl Value {
    fn id(&self) -> OrbitalId {
        match self {
            Self::Int(orbital) => orbital.id(),
            Self::Float(orbital) => orbital.id(),
            Self::Bool(orbital) => orbital.id(),
        }
    }

    /// The id says which type it names, so opening is one typed open.
    fn open(
        orbitals: &Orbitals,
        id: OrbitalId,
    ) -> Result<Self, PhpException> {
        match id.kind() {
            Kind::I64 => orbitals.open(id).map(Self::Int),
            Kind::F64 => orbitals.open(id).map(Self::Float),
            Kind::Bool => orbitals.open(id).map(Self::Bool),
            Kind::U64 => {
                return Err(failure(
                    "a u64 orbital has no PHP type; PHP integers are signed",
                ));
            }
        }
        .map_err(failure)
    }

    fn parse(id: &str) -> Result<Self, PhpException> {
        let id: OrbitalId = id.parse().map_err(failure)?;
        Self::open(&bridge()?.orbitals, id)
    }

    fn load(&self) -> Loaded {
        match self {
            Self::Int(orbital) => Loaded::Int(orbital.load()),
            Self::Float(orbital) => Loaded::Float(orbital.load()),
            Self::Bool(orbital) => Loaded::Bool(orbital.load()),
        }
    }

    fn store(
        &self,
        value: &Zval,
    ) -> Result<(), PhpException> {
        match self {
            Self::Int(orbital) => orbital.store(expect_long(value)?),
            Self::Float(orbital) => orbital.store(expect_double(value)?),
            Self::Bool(orbital) => orbital.store(expect_bool(value)?),
        }
        Ok(())
    }

    /// `+=` / `-=` / `++` / `--`: atomic, wrapping for an int, and the
    /// value after it.
    fn add(
        &self,
        by: &Zval,
        negate: bool,
    ) -> Result<Loaded, PhpException> {
        match self {
            Self::Int(orbital) => {
                let by = expect_long(by)?;
                Ok(Loaded::Int(if negate {
                    orbital.fetch_sub(by).wrapping_sub(by)
                } else {
                    orbital.fetch_add(by).wrapping_add(by)
                }))
            }
            Self::Float(orbital) => {
                let by = expect_double(by)?;
                let by = if negate { -by } else { by };
                Ok(Loaded::Float(orbital.fetch_add(by) + by))
            }
            Self::Bool(_) => Err(failure("a bool orbital has no arithmetic")),
        }
    }

    /// The value now, when it is no longer `current`. Compared as the wait
    /// compares: a float by its bits.
    fn changed(
        &self,
        current: &Zval,
    ) -> Result<Option<Loaded>, PhpException> {
        let now = self.load();
        let same = match now {
            Loaded::Int(now) => now == expect_long(current)?,
            Loaded::Float(now) => {
                now.to_bits() == expect_double(current)?.to_bits()
            }
            Loaded::Bool(now) => now == expect_bool(current)?,
        };
        Ok((!same).then_some(now))
    }

    /// Park until the value is no longer `current`; `None` is the interrupt.
    fn wait_until(
        &self,
        current: &Zval,
        interrupt: &AtomicBool,
    ) -> Result<Option<Loaded>, PhpException> {
        Ok(match self {
            Self::Int(orbital) => orbital
                .wait_until(expect_long(current)?, interrupt)
                .map_err(failure)?
                .map(Loaded::Int),
            Self::Float(orbital) => orbital
                .wait_until(expect_double(current)?, interrupt)
                .map_err(failure)?
                .map(Loaded::Float),
            Self::Bool(orbital) => orbital
                .wait_until(expect_bool(current)?, interrupt)
                .map_err(failure)?
                .map(Loaded::Bool),
        })
    }

    fn notify(&self) {
        match self {
            Self::Int(orbital) => orbital.notify(),
            Self::Float(orbital) => orbital.notify(),
            Self::Bool(orbital) => orbital.notify(),
        }
    }

    /// [`Self::wait_until`], ended by this execution's cancellation.
    ///
    /// The wait parks on the value's own word, so cancelling is a wake, not
    /// a deadline: a task on the runtime awaits the token, sets the
    /// interrupt and then notifies the value, in that order.
    fn wait(
        &self,
        current: &Zval,
    ) -> Result<Option<Loaded>, PhpException> {
        // A change made while PHP was not waiting needs no canceller.
        if let Some(now) = self.changed(current)? {
            return Ok(Some(now));
        }
        let cancellation = cancellation();
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let interrupt = Arc::new(AtomicBool::new(false));
        let canceller = bridge()?.runtime.spawn({
            let value = self.clone();
            let interrupt = Arc::clone(&interrupt);
            async move {
                cancellation.cancelled().await;
                interrupt.store(true, MemoryOrdering::SeqCst);
                value.notify();
            }
        });
        let result = self.wait_until(current, &interrupt);
        canceller.abort();
        result
    }
}

#[derive(Clone, Copy)]
enum Loaded {
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Loaded {
    fn write(
        self,
        target: &mut Zval,
    ) {
        match self {
            Self::Int(value) => target.set_long(value),
            Self::Float(value) => target.set_double(value),
            Self::Bool(value) => target.set_bool(value),
        }
    }

    fn into_zval(self) -> Zval {
        let mut value = Zval::new();
        self.write(&mut value);
        value
    }

    fn as_string(self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Bool(true) => "1".to_owned(),
            Self::Bool(false) => String::new(),
        }
    }

    fn as_f64(self) -> f64 {
        match self {
            Self::Int(value) => value as f64,
            Self::Float(value) => value,
            Self::Bool(value) => f64::from(u8::from(value)),
        }
    }

    fn compare(
        self,
        other: &Zval,
    ) -> Option<Ordering> {
        let theirs = other
            .double()
            .or_else(|| other.long().map(|value| value as f64))
            .or_else(|| other.bool().map(|value| f64::from(u8::from(value))))?;
        self.as_f64().partial_cmp(&theirs)
    }
}

fn expect_long(value: &Zval) -> Result<i64, PhpException> {
    value.long().ok_or_else(|| failure("an int orbital takes an int"))
}

fn expect_double(value: &Zval) -> Result<f64, PhpException> {
    value
        .double()
        .or_else(|| value.long().map(|value| value as f64))
        .ok_or_else(|| failure("a float orbital takes a number"))
}

fn expect_bool(value: &Zval) -> Result<bool, PhpException> {
    value.bool().ok_or_else(|| failure("a bool orbital takes a bool"))
}

/// The PHP handle. `Final` and no dynamic properties: the value is not on
/// the object, it is in the fleet.
#[php_class]
#[php(name = "Claviron\\Runtime\\Orbital")]
#[php(flags = ClassFlags::from_bits_retain(ClassFlags::Final.bits() | ClassFlags::NoDynamicProperties.bits()))]
pub(crate) struct PhpOrbital {
    value: Option<Value>,
}

impl PhpOrbital {
    fn value(&self) -> Result<&Value, PhpException> {
        self.value
            .as_ref()
            .ok_or_else(|| failure("this handle is not attached to an orbital"))
    }

    fn attached(value: Value) -> ZBox<ZendClassObject<Self>> {
        ZendClassObject::new(Self { value: Some(value) })
    }
}

#[php_impl]
impl PhpOrbital {
    #[php(defaults(value = 0))]
    pub fn int(
        value: i64
    ) -> Result<ZBox<ZendClassObject<PhpOrbital>>, PhpException> {
        Ok(PhpOrbital::attached(Value::Int(
            bridge()?.orbitals.allocate(value).map_err(failure)?,
        )))
    }

    #[php(defaults(value = 0.0))]
    pub fn float(
        value: f64
    ) -> Result<ZBox<ZendClassObject<PhpOrbital>>, PhpException> {
        Ok(PhpOrbital::attached(Value::Float(
            bridge()?.orbitals.allocate(value).map_err(failure)?,
        )))
    }

    #[php(defaults(value = false))]
    pub fn bool(
        value: bool
    ) -> Result<ZBox<ZendClassObject<PhpOrbital>>, PhpException> {
        Ok(PhpOrbital::attached(Value::Bool(
            bridge()?.orbitals.allocate(value).map_err(failure)?,
        )))
    }

    pub fn open(
        id: String
    ) -> Result<ZBox<ZendClassObject<PhpOrbital>>, PhpException> {
        Ok(PhpOrbital::attached(Value::parse(&id)?))
    }

    pub fn id(&self) -> Result<String, PhpException> {
        Ok(self.value()?.id().to_string())
    }

    pub fn get(&self) -> Result<Zval, PhpException> {
        Ok(self.value()?.load().into_zval())
    }

    pub fn set(
        &self,
        value: &Zval,
    ) -> Result<(), PhpException> {
        self.value()?.store(value)
    }

    pub fn add(
        &self,
        by: &Zval,
    ) -> Result<Zval, PhpException> {
        Ok(self.value()?.add(by, false)?.into_zval())
    }

    /// Block until the value is no longer `$current`, whichever process
    /// writes it, and return what it is then. Returns at once when it
    /// already differs. Coalescing: many writes while parked wake once and
    /// the latest value is returned. Null means this execution was
    /// cancelled, which `claviron_is_cancelled()` then reports.
    pub fn wait(
        &self,
        current: &Zval,
    ) -> Result<Zval, PhpException> {
        Ok(match self.value()?.wait(current)? {
            Some(value) => value.into_zval(),
            None => Zval::new(),
        })
    }

    pub fn __serialize(&self) -> Result<Vec<(String, String)>, PhpException> {
        Ok(vec![("id".to_owned(), self.value()?.id().to_string())])
    }

    pub fn __unserialize(
        &mut self,
        data: &ZendHashTable,
    ) -> Result<(), PhpException> {
        let id = data
            .get("id")
            .and_then(Zval::string)
            .ok_or_else(|| failure("serialized form carries no id"))?;
        self.value = Some(Value::parse(&id)?);
        Ok(())
    }

    pub fn __to_string(&self) -> Result<String, PhpException> {
        Ok(self.value()?.load().as_string())
    }
}

/// Patch the class's own handler table, once, after registration. The table
/// keeps its identity, which is what ext-php-rs uses to recognise its
/// objects; only three entries change.
pub(crate) fn install_handlers() {
    let metadata = PhpOrbital::get_metadata();
    let handlers = metadata.handlers() as *const _
        as *mut ext_php_rs::zend::ZendObjectHandlers;
    unsafe {
        (*handlers).do_operation = Some(do_operation);
        (*handlers).compare = Some(compare);
        (*handlers).cast_object = Some(cast_object);
        // ext-php-rs denies serialization for every class with a Rust body,
        // because it cannot know what the body means. This one can:
        // `__serialize` carries the id and nothing else.
        let class =
            metadata.ce() as *const _ as *mut ext_php_rs::zend::ClassEntry;
        (*class).ce_flags &= !ext_php_rs::ffi::ZEND_ACC_NOT_SERIALIZABLE;
        // `unserialize()` builds the object without a constructor and then
        // calls `__unserialize` on it. ext-php-rs leaves such an object's
        // Rust body uninitialised until a constructor runs; this class has
        // an honest empty state, so its objects start there.
        (*class).__bindgen_anon_2.create_object = Some(create_object);
    }
}

/// A detached handle; `__unserialize` or a factory attaches it.
unsafe extern "C" fn create_object(
    _class: *mut ext_php_rs::zend::ClassEntry
) -> *mut zend_object {
    let object = ZendClassObject::new(PhpOrbital { value: None });
    let raw = ZBox::into_raw(object);
    // SAFETY: into_raw transfers a live ZendClassObject with its embedded header.
    unsafe { &raw mut (*raw).std }
}

unsafe fn orbital_of<'a>(value: *mut zval) -> Option<&'a PhpOrbital> {
    let object: &ZendObject = unsafe { (*value).object()? };
    let raw: &zend_object = object;
    ZendClassObject::<PhpOrbital>::from_zend_obj(raw).map(|wrapped| &**wrapped)
}
/// `$n + 1`, `$n += 1`, `$n++`, `$n - 1`, `$n -= 1`, `$n--`: applied to the
/// shared value, result is the same handle. Anything else is not an
/// operation an Orbital has, and PHP reports the unsupported operand types
/// itself.
unsafe extern "C" fn do_operation(
    opcode: u8,
    result: *mut zval,
    op1: *mut zval,
    op2: *mut zval,
) -> c_int {
    let Some(orbital) = (unsafe { orbital_of(op1) }) else {
        return ZEND_RESULT_CODE_FAILURE;
    };
    let Ok(value) = orbital.value() else {
        return ZEND_RESULT_CODE_FAILURE;
    };
    let operand: &Zval = unsafe { &*op2 };
    let applied = match opcode {
        ZEND_ADD => value.add(operand, false),
        ZEND_SUB => value.add(operand, true),
        _ => return ZEND_RESULT_CODE_FAILURE,
    };
    if let Err(exception) = applied {
        exception.throw();
        return ZEND_RESULT_CODE_FAILURE;
    }
    if result != op1 {
        unsafe { std::ptr::write(result, (*op1).shallow_clone()) };
    }
    ZEND_RESULT_CODE_SUCCESS
}

unsafe fn loaded_of(value: *mut zval) -> Option<Option<Loaded>> {
    let orbital = unsafe { orbital_of(value) }?;
    Some(orbital.value().ok().map(Value::load))
}

unsafe extern "C" fn compare(
    op1: *mut zval,
    op2: *mut zval,
) -> c_int {
    let ordering = if let Some(mine) = unsafe { loaded_of(op1) } {
        mine.and_then(|mine| match unsafe { loaded_of(op2) } {
            Some(theirs) => theirs
                .and_then(|theirs| mine.as_f64().partial_cmp(&theirs.as_f64())),
            None => mine.compare(unsafe { &*op2 }),
        })
    } else if let Some(theirs) = unsafe { loaded_of(op2) } {
        theirs
            .and_then(|theirs| theirs.compare(unsafe { &*op1 }))
            .map(Ordering::reverse)
    } else {
        None
    };
    match ordering {
        Some(Ordering::Less) => -1,
        Some(Ordering::Equal) => 0,
        Some(Ordering::Greater) => 1,
        // Incomparable: PHP's own convention for objects that do not compare.
        None => 1,
    }
}

unsafe extern "C" fn cast_object(
    object: *mut zend_object,
    retval: *mut zval,
    kind: c_int,
) -> c_int {
    let raw: &zend_object = unsafe { &*object };
    let Some(orbital) = ZendClassObject::<PhpOrbital>::from_zend_obj(raw)
        .map(|wrapped| &**wrapped)
    else {
        return ZEND_RESULT_CODE_FAILURE;
    };
    let Ok(value) = orbital.value().map(Value::load) else {
        return ZEND_RESULT_CODE_FAILURE;
    };
    // PHP hands over an uninitialised stack zval. ext-php-rs setters release
    // whatever the zval held before, so it must hold nothing first.
    unsafe { retval.write(Zval::new()) };
    let target: &mut Zval = unsafe { &mut *retval };
    match kind {
        IS_LONG => target.set_long(match value {
            Loaded::Int(value) => value,
            Loaded::Float(value) => value as i64,
            Loaded::Bool(value) => i64::from(value),
        }),
        IS_DOUBLE => target.set_double(value.as_f64()),
        IS_STRING => {
            if target.set_string(&value.as_string(), false).is_err() {
                return ZEND_RESULT_CODE_FAILURE;
            }
        }
        IS_BOOL => target.set_bool(match value {
            Loaded::Int(value) => value != 0,
            Loaded::Float(value) => value != 0.0,
            Loaded::Bool(value) => value,
        }),
        _ => return ZEND_RESULT_CODE_FAILURE,
    }
    ZEND_RESULT_CODE_SUCCESS
}

#[cfg(test)]
#[path = "../../../tests/functions/orbit/atomic.rs"]
mod tests;
