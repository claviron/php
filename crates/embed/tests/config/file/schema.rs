use super::*;

#[test]
fn warmup_folders_use_the_common_engine_schema() {
    let config = parse(
        "engine: {warmup: [app, vendor/example/package]}\nsapi: null\n",
        |_| None,
    )
    .unwrap()
    .resolve()
    .unwrap();
    assert_eq!(
        config.engine.warmup,
        vec![PathBuf::from("app"), PathBuf::from("vendor/example/package"),]
    );
    assert!(parse("engine: {warmup: app}\nsapi: null\n", |_| None).is_err());
    assert!(parse("engine: {validate_timestamps: false}\n", |_| None).is_err());
}

#[test]
fn event_backlog_is_configurable_and_invalid_bounds_are_rejected() {
    let config = parse("sapi: null\nevents: {capacity: 128}\n", |_| None)
        .unwrap()
        .resolve()
        .unwrap();
    assert_eq!(config.events.capacity, 128);
    assert!(
        parse("sapi: null\nevents: {capacity: 0}\n", |_| None)
            .unwrap()
            .resolve()
            .is_err()
    );
    assert!(parse("sapi: null\nevents: {capacitty: 128}\n", |_| None).is_err());
    assert!(crate::PhpEventConfig { capacity: usize::MAX }.validate().is_err());
}

#[test]
fn single_application_uses_the_embed_schema_without_a_second_name_or_pool() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config/program.php");
    let raw = format!(
        "application: {{script_path: {}}}\n",
        serde_yaml::to_string(&script).unwrap().trim()
    );
    let config = parse_application(&raw, "example-app", |_| None).unwrap();
    let resolved = config.resolve().unwrap();
    assert!(resolved.sapi.is_none());
    assert_eq!(resolved.applications.len(), 1);
    assert_eq!(resolved.applications[0].name, "example-app");
    assert!(
        parse_application(
            "applications: {}\napplication: {}\n",
            "example",
            |_| None
        )
        .is_err()
    );
    assert!(
        parse_application("application: {unknown: true}\n", "example", |_| {
            None
        })
        .is_err()
    );
}

#[test]
fn persistent_application_rejects_the_retired_elastic_pool_schema() {
    assert!(parse_application(
        "application: {script_path: application.php, pool: {min_idle: 1, max_size: 8}}",
        "example", |_| None,
    ).is_err());
}

#[test]
fn linked_application_uses_its_instances_without_an_engine_queue() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config/program.php");
    let raw = format!(
        "application: {{script_path: {}, instances: 8}}",
        serde_yaml::to_string(&script).unwrap().trim()
    );
    let mut config = parse_application(&raw, "example", |_| None).unwrap();
    config.use_link_queue("example").unwrap();
    let resolved = config.resolve().unwrap();
    assert_eq!(resolved.applications[0].admission_capacity(), Some(8));

    let raw = raw.replace("instances: 8", "instances: 8, max_queued: 4");
    let mut config = parse_application(&raw, "example", |_| None).unwrap();
    assert!(
        config
            .use_link_queue("example")
            .unwrap_err()
            .to_string()
            .contains("use orbit.link.max_queued")
    );
    assert_eq!(
        config.resolve().unwrap().applications[0].admission_capacity(),
        Some(12)
    );

    let raw = raw.replace("max_queued: 4", "max_waiting: null");
    let error = parse_application(&raw, "example", |_| None).unwrap_err();
    assert!(
        error.to_string().contains("max_queued counts only waiting exchanges")
    );
}
