//! PHP execution engines and configured pipeline owners.
//!
//! Select `embed` or `fcgi`. Add `backend` to prepare PHP directives through
//! a host-independent `claviron_pipeline::SiteRegistry`.

#[cfg(any(feature = "embed", feature = "fcgi"))]
pub use claviron_php_core::{PhpConfig, PhpRunner, PhpRuntime, headers};
#[cfg(feature = "embed")]
pub use claviron_php_embed as embed;
#[cfg(feature = "fcgi")]
pub use claviron_php_fcgi as fcgi;
#[cfg(all(feature = "backend", any(feature = "embed", feature = "fcgi")))]
pub mod backend;

/// Register the selected engines in the application's lifecycle registry.
/// With `edge`, also install the `php:` route factory on Edge.
#[cfg(any(feature = "embed", feature = "fcgi"))]
pub fn register(
    state: &claviron_runtime::AppState
) -> claviron_runtime::registry::Result<()> {
    #[cfg(feature = "fcgi")]
    fcgi::register(state)?;
    #[cfg(feature = "embed")]
    embed::register(state)?;
    #[cfg(feature = "edge")]
    if let Some(hosts) = state.registry_ref().resolve::<claviron_edge::HostAppRegistry>() {
        backend::register(hosts.site())?;
    }
    Ok(())
}
