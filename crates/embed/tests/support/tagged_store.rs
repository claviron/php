//! A minimal store with tags, owned by the tests: what they check is the PHP
//! bridge, its arguments and the exception class each refusal reaches PHP
//! as, not a driver's tag semantics.
//!
//! Its fence is the contract's in its simplest form: one clock that every
//! invalidation advances and records as the floor of each tag it names. A
//! fill is refused when one of its tags has a floor above its token.

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use claviron_cache::{
    CacheStore, Error, Fill, FillToken, Result, TaggedCacheStore,
};

/// Tags per entry and per invalidation; the tag fixture exceeds it.
pub const MAX_TAGS: usize = 2;

/// The clock this store issues tokens from.
const SOURCE: u64 = 7;

#[derive(Default)]
pub struct TaggedStore {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, (Bytes, Vec<String>)>,
    clock: u64,
    floors: HashMap<String, u64>,
}

impl TaggedStore {
    fn insert(
        &self,
        key: &str,
        value: Cow<'_, [u8]>,
        tags: &[&str],
    ) {
        let tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        self.state
            .lock()
            .unwrap()
            .entries
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
        Ok(self
            .state
            .lock()
            .unwrap()
            .entries
            .get(key)
            .map(|entry| entry.0.clone()))
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
        self.state.lock().unwrap().entries.remove(key);
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

    async fn fill_token(&self) -> Result<FillToken> {
        Ok(FillToken::new(SOURCE, self.state.lock().unwrap().clock))
    }

    async fn fill_tagged(
        &self,
        token: FillToken,
        key: &str,
        value: Cow<'_, [u8]>,
        _ttl: Option<Duration>,
        tags: &[&str],
    ) -> Result<Fill> {
        if tags.len() > MAX_TAGS {
            return Err(Error::TooManyTags {
                count: tags.len(),
                max: MAX_TAGS,
            });
        }
        {
            let state = self.state.lock().unwrap();
            let newer = tags.iter().any(|tag| {
                state
                    .floors
                    .get(*tag)
                    .is_some_and(|&floor| floor > token.clock())
            });
            if token.source() != SOURCE || newer {
                return Ok(Fill::Invalidated);
            }
        }
        self.insert(key, value, tags);
        Ok(Fill::Kept)
    }

    async fn invalidate_tags(
        &self,
        tags: &[&str],
    ) -> Result<()> {
        if tags.is_empty() {
            return Err(Error::TagsEmpty);
        }
        if tags.len() > MAX_TAGS {
            return Err(Error::TooManyTags {
                count: tags.len(),
                max: MAX_TAGS,
            });
        }
        let mut state = self.state.lock().unwrap();
        state.clock += 1;
        let clock = state.clock;
        for tag in tags {
            state.floors.insert((*tag).to_owned(), clock);
        }
        state.entries.retain(|_, (_, kept)| {
            !kept.iter().any(|tag| tags.contains(&tag.as_str()))
        });
        Ok(())
    }
}
