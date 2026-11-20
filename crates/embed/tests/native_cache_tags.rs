//! The PHP tag functions against a store with tags and the built-in memory
//! store without them.

#[path = "support/native_runtime.rs"]
mod native_runtime;

#[path = "support/tagged_store.rs"]
mod tagged_store;

use std::sync::Arc;

use claviron_cache::{CacheConfig, CacheProvider, CacheRegistry};
use tagged_store::TaggedStore;

#[tokio::test]
async fn tag_functions_reach_php_with_their_own_exceptions() {
    native_runtime::run_fixture(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/functions/cache_tags.php"),
        async |state| {
            // The default store is the built-in memory store, without tags.
            claviron_cache::register_with_config(state, CacheConfig::default())
                .unwrap();
            let registry = state.registry_ref();
            registry
                .resolve::<CacheRegistry>()
                .unwrap()
                .register_store(Arc::new(TaggedStore::default()))
                .unwrap();
            let provider = registry.resolve::<CacheProvider>().unwrap();
            claviron_runtime::AnyComponent::boot(
                provider.as_ref(),
                registry,
                state,
            )
            .await
            .unwrap();
        },
    )
    .await;
}
