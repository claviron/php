use super::*;

#[test]
fn wait_returns_the_changed_value_and_ends_on_the_interrupt() {
    let orbitals = Orbitals::new(Arc::new(
        claviron_orbit::Fleet::join("orbital-wait", 1).unwrap(),
    ))
    .unwrap();
    let shared = orbitals.allocate(7_i64).unwrap();
    // The id alone says the value is an int.
    let Ok(handle) = Value::open(&orbitals, shared.id()) else {
        panic!("an int id opens");
    };
    let interrupt = AtomicBool::new(false);
    let mut seen = Zval::new();
    seen.set_long(7);
    shared.store(8);
    assert!(matches!(
        handle.wait_until(&seen, &interrupt),
        Ok(Some(Loaded::Int(8)))
    ));
    // Unchanged, the wait ends only because the flag was set before the wake.
    seen.set_long(8);
    interrupt.store(true, MemoryOrdering::SeqCst);
    handle.notify();
    assert!(matches!(handle.wait_until(&seen, &interrupt), Ok(None)));
}
