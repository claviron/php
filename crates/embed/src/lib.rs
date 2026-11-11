#[cfg(feature = "backend")]
pub mod backend;
use claviron_runtime::AppState;

#[cfg(feature = "auth")]
pub mod auth;
mod config;
pub mod console;
mod engine;
mod error;
pub mod evaluate;
mod event;
pub mod extension;
pub mod framework;
mod functions;
mod intercept;
mod invalidation;
mod manager;
#[cfg(feature = "metrics")]
mod metrics;
pub mod observer;
#[cfg(feature = "postgres")]
pub mod postgres;
mod routing;
mod sapi;
mod service;
#[cfg(feature = "tasks")]
pub(crate) mod tasks;
mod warmup;

pub use config::{
    PhpApplication, PhpEventConfig, PhpLaunchConfig, PhpManagementConfig,
    RequestPool,
};
pub use engine::application::PhpApplicationSnapshot;
pub use engine::{
    PhpSapiEngine, PhpSapiRequest, SapiThreadPoolSnapshot,
    SapiThreadSlotSnapshot,
};
pub use error::SapiGatewayError;
pub use invalidation::ipc::{
    PhpCacheInvalidationKind, PhpCacheInvalidationObserved,
};
#[cfg(any(feature = "apcu-intercept", feature = "opcache-intercept"))]
use invalidation::{bus, ipc};
pub use manager::{
    PhpApplicationController, PhpManagerError, PhpManagerResult,
};
#[cfg(feature = "metrics")]
pub use metrics::PhpEmbedMetricsConfig;
pub use routing::adapter::application::PhpApplicationAdapter;
pub use routing::adapter::script::PhpScriptAdapter;
pub use service::{
    PhpApplicationHandle, PhpApplicationLifecycle, PhpApplicationService,
    PhpApplicationStatus, PhpEmbedRuntime, PhpEmbedRuntimeBuilder,
    PhpEmbedStateExt, PhpScriptService,
};

/// Register PHP from `php.yaml` in the runtime configuration directory.
/// The library owns parsing, environment-file resolution, and validation.
/// A missing file preserves the default one-thread request pool.
/// Use [`PhpEmbedRuntime::builder`] for explicit, configuration-free composition.
/// An existing runtime keeps its settings; repeated boot-time registration
/// completes optional task bindings after TaskService becomes available.
pub fn register(state: &AppState) -> claviron_runtime::registry::Result<()> {
    if let Some(runtime) = state.registry_ref().resolve::<PhpEmbedRuntime>() {
        // A custom runtime may precede TaskService during server definitions.
        // Complete optional wiring here without replacing its PHP settings.
        #[cfg(feature = "tasks")]
        tasks::register(state, runtime)
            .map_err(claviron_runtime::registry::Error::msg)?;
        #[cfg(not(feature = "tasks"))]
        let _ = runtime;
        return Ok(());
    }
    config::file::load_service(state)?.register(state)
}

/// Host one application using the embed schema from an enclosing launch file.
/// The host supplies its name; `application` is the usual per-application
/// definition, beside `engine`, `lifecycle` and the other embed policies.
pub fn register_application_from_yaml(
    state: &AppState,
    name: &str,
    yaml: &str,
    description: claviron_app::ApplicationDescription,
) -> claviron_runtime::registry::Result<()> {
    config::file::load_application(state, name, yaml, description, false)?
        .register(state)
}

/// Validate the owner of a linked PHP application's execution and queue bounds.
pub fn linked_application_instances(
    php: &serde_yaml::Value,
    orbit: Option<&serde_yaml::Value>,
) -> claviron_runtime::registry::Result<Option<usize>> {
    use claviron_runtime::registry::Error;

    let Some(link) = orbit
        .and_then(|orbit| orbit.get("link"))
        .filter(|link| !link.is_null())
    else {
        return Ok(None);
    };
    let application = php.get("application");
    if application.is_some_and(|app| app.get("max_waiting").is_some()) {
        return Err(Error::msg(
            "php.application.max_waiting was removed; orbit.link.max_queued counts only waiting exchanges",
        ));
    }
    if application.is_some_and(|app| app.get("max_queued").is_some()) {
        return Err(Error::msg(
            "php.application.max_queued is owned by Link; use orbit.link.max_queued",
        ));
    }
    if link.get("max_in_flight").is_some() {
        return Err(Error::msg(
            "orbit.link.max_in_flight is set by php.application.instances; remove orbit.link.max_in_flight",
        ));
    }
    let instances = application
        .and_then(|app| app.get("instances"))
        .filter(|value| !value.is_null())
        .map(|value| serde_yaml::from_value::<u8>(value.clone()))
        .transpose()
        .map_err(|error| {
            Error::msg(format!("php.application.instances: {error}"))
        })?
        .unwrap_or_else(config::default_application_instances);
    if instances == 0 {
        return Err(Error::msg("php.application.instances must be at least 1"));
    }
    Ok(Some(usize::from(instances)))
}

/// Register a persistent PHP application behind Link. Link owns its queue.
pub fn register_linked_application_from_yaml(
    state: &AppState,
    name: &str,
    yaml: &str,
    description: claviron_app::ApplicationDescription,
) -> claviron_runtime::registry::Result<()> {
    config::file::load_application(state, name, yaml, description, true)?
        .register(state)
}

/// Host one script-mode application using the existing embed engine and SAPI
/// pool schema from an enclosing launch document. The request route is owned
/// by the caller, not this engine configuration.
pub fn register_script_from_yaml(
    state: &AppState,
    name: &str,
    yaml: &str,
    description: claviron_app::ApplicationDescription,
) -> claviron_runtime::registry::Result<()> {
    config::file::load_script(state, name, yaml, description)?.register(state)
}

#[cfg(test)]
#[path = "../tests/support.rs"]
mod test_support;
