//! Cold PHP composition for a host-independent site registry.

use crate::{PhpConfig, PhpRunner, PhpRuntime};
use claviron_pipeline::{
    BackendHandler, Site, SiteRegistry,
    factory::{BackendContext, BackendFactory, BackendPlacement},
};
use claviron_runtime::{
    AppState,
    registry::{Error, Result},
};
use std::sync::Arc;

/// Install PHP preparation for this site registry. Both Edge and a single-site
/// server can supply the registry; engine registration remains separate.
pub fn register(registry: &SiteRegistry) -> Result<()> {
    registry.register_backend(Arc::new(PhpFactory))
}

struct PhpFactory;
impl BackendFactory for PhpFactory {
    fn name(&self) -> &'static str {
        "php"
    }
    fn placement(&self) -> BackendPlacement {
        BackendPlacement::AfterFiles
    }
    fn prepare(
        &self,
        context: &BackendContext<'_>,
    ) -> Result<Vec<Arc<dyn BackendHandler>>> {
        let declared = declarations(context.route).map_err(|error| {
            Error::msg(format!(
                "backend 'php' at '{}': {error}",
                context.prefix
            ))
        })?;
        // Every declaration in this scope shares its resolved prefix. Retain
        // last-declared precedence without another runtime mount dispatcher.
        prepare_runtimes(context.state, context.route, &declared)
            .into_iter()
            .rev()
            .map(|runtime| match runtime.php_runner {
                #[cfg(feature = "embed")]
                PhpRunner::Script => crate::embed::backend::prepare_script(
                    context.state,
                    runtime,
                ),
                #[cfg(feature = "embed")]
                PhpRunner::Application => {
                    Ok(crate::embed::backend::prepare_application(
                        context.state,
                        runtime,
                    ))
                }
                #[cfg(feature = "fcgi")]
                PhpRunner::Fcgi => {
                    crate::fcgi::backend::prepare(context.state, runtime)
                }
            })
            .collect()
    }
}

/// The route's `php:` directive as declared: one mapping or a list of them.
/// The edge keeps it as written; this is where it is decoded and validated.
/// An absent or null directive declares nothing.
fn declarations(
    route: &Site
) -> std::result::Result<Vec<PhpConfig>, serde_yaml::Error> {
    match route.directive("php") {
        None | Some(serde_yaml::Value::Null) => Ok(Vec::new()),
        Some(serde_yaml::Value::Sequence(items)) => {
            items.iter().cloned().map(serde_yaml::from_value).collect()
        }
        Some(one) => Ok(vec![serde_yaml::from_value(one.clone())?]),
    }
}

// Resolve route mappings and PHP defaults before selecting runner owners.
fn prepare_runtimes(
    state: &AppState,
    config: &Site,
    declared: &[PhpConfig],
) -> Vec<PhpRuntime> {
    declared
        .iter()
        .map(|template| {
            let mut runtime = PhpRuntime::from_config(
                state,
                template,
                config.files.mapping.as_ref().map_or_else(
                    || config.root(),
                    |mapping| mapping.directory().to_path_buf(),
                ),
                config.indexes(),
            );
            runtime.path = config
                .files
                .mapping
                .as_ref()
                .map(|mapping| mapping.prefix().clone());
            if let Some(mapping) = &config.files.mapping {
                runtime.mapping =
                    Some(claviron_http::config::mapping::RouteMapping::new(
                        mapping.prefix().clone(),
                        runtime.dir.clone(),
                    ));
            }
            runtime
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/backend.rs"]
mod tests;
