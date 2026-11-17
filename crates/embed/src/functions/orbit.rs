//! PHP functions backed by Orbit primitives.

pub(crate) mod atomic;
pub(crate) mod counter;
#[cfg(feature = "tasks")]
pub(crate) mod invoke;
pub(crate) mod lock;
