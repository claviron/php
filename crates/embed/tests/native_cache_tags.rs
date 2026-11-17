//! The PHP tag functions against a store with tags and the built-in memory
//! store without them. The tagged store here is a minimal one owned by this
//! test: what is checked is the bridge, its arguments and the exception
//! class each refusal reaches PHP as, not a driver's tag semantics.

#[path = "support/native_runtime.rs"]
mod native_runtime;

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use claviron_cache::{
    CacheConfig, CacheProvider, CacheRegistry, CacheStore, Error, Result,
    TaggedCacheStore,
};

/// Tags per entry and per invalidation; the fixture exceeds it on purpose.
const MAX_TAGS: usize = 2;

#[derive(Default)]
struct TaggedStore {
    entries: Mutex<HashMap<String, (Bytes, Vec<String>)>>,
}

impl TaggedStore {
    fn insert(
        &self,
        key: &str,
        value: Cow<'_, [u8]>,
        tags: &[&str],
    ) {
        let tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_owned(), (Bytes::from(value.into_owned()), tags));
    }
}

#[async_trait]
impl CacheStore for TaggedStore {
    fn store_name(&self) -> &str {
        "tagged"
    }

    fn driver_name(&self) -> &str {
        "test"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_ttl(&self) -> bool {
        false
    }

    async fn get(
        &self,
        key: &str,
    ) -> Result<Option<Bytes>> {
        Ok(self.entries.lock().unwrap().get(key).map(|entry| entry.0.clone()))
    }

    async fn put(
        &self,
        key: &str,
        value: Cow<'_, [u8]>,
        _ttl: Option<Duration>,
    ) -> Result<()> {
        self.insert(key, value, &[]);
        Ok(())
    }

    async fn forget(
        &self,
        key: &str,
    ) -> Result<()> {
        self.entries.lock().unwrap().remove(key);
        Ok(())
    }

    fn tagged(&self) -> Option<&dyn TaggedCacheStore> {
        Some(self)
    }
}

#[async_trait]
impl TaggedCacheStore for TaggedStore {
    async fn put_tagged(
        &self,
        key: &str,
        value: Cow<'_, [u8]>,
        _ttl: Option<Duration>,
        tags: &[&str],
    ) -> Result<()> {
        if tags.len() > MAX_TAGS {
            return Err(Error::TooManyTags {
                count: tags.len(),
                max: MAX_TAGS,
            });
        }
        self.insert(key, value, tags);
        Ok(())
    }

    async fn invalidate_tags(
        &self,
        tags: &[&str],
    ) -> Result<()> {
        if tags.is_empty() {
            return Err(Error::TagsEmpty);
        }
        self.entries.lock().unwrap().retain(|_, (_, kept)| {
            !kept.iter().any(|tag| tags.contains(&tag.as_str()))
        });
        Ok(())
    }
}

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
