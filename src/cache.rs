use crate::lazy::LazyPayload;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, RwLock, Weak},
};

pub(crate) trait SingleTypeCacheObj: Send + Sync {
    fn get_or_insert_with(
        &self,
        identity: u64,
        create_fn: &dyn Fn() -> Arc<dyn Any + Send + Sync + 'static>,
    ) -> Arc<dyn Any + Send + Sync + 'static>;
}

pub(crate) struct SingleTypeCache {
    pub(crate) values: RwLock<HashMap<u64, Weak<LazyPayload>>>,
}

impl SingleTypeCache {
    fn new() -> Self {
        Self {
            values: Default::default(),
        }
    }
}

impl SingleTypeCacheObj for SingleTypeCache {
    fn get_or_insert_with<'a, 'b>(
        &self,
        identity: u64,
        create_fn: &dyn Fn() -> Arc<dyn Any + Send + Sync + 'static>,
    ) -> Arc<dyn Any + Send + Sync + 'static> {
        let values = self.values.read().unwrap();
        if let Some(existing) = values.get(&identity).and_then(Weak::upgrade) {
            return existing;
        }

        // Lock mutably instead.
        drop(values);
        let mut values = self.values.write().unwrap();

        match values.entry(identity) {
            std::collections::hash_map::Entry::Occupied(mut existing) => {
                if let Some(existing) = existing.get().upgrade() {
                    // Entry exists and is still valid
                    existing
                } else {
                    // Entry exists, but the weak pointer is dead. Re-create.
                    let res = Arc::downcast::<LazyPayload>((create_fn)()).unwrap();
                    *existing.get_mut() = Arc::downgrade(&res);
                    res
                }
            }
            std::collections::hash_map::Entry::Vacant(vacant) => {
                // Entry does not exist
                let res = Arc::downcast::<LazyPayload>((create_fn)()).unwrap();
                vacant.insert(Arc::downgrade(&res));
                res
            }
        }
    }
}

pub struct Cache {
    pub(crate) typed_caches: RwLock<HashMap<TypeId, Arc<dyn SingleTypeCacheObj>>>,
}

impl Cache {
    pub fn create() -> Arc<Cache> {
        Arc::new(Self {
            typed_caches: Default::default(),
        })
    }

    pub(crate) fn get_or_insert_with(
        &self,
        type_id: TypeId,
        identity: u64,
        create_fn: impl (FnOnce() -> LazyPayload) + 'static,
    ) -> Arc<LazyPayload> {
        let typed_cache_obj = {
            let typed_caches = self.typed_caches.read().unwrap();
            if !typed_caches.contains_key(&type_id) {
                drop(typed_caches);
                let mut typed_caches = self.typed_caches.write().unwrap();
                typed_caches
                    .entry(type_id)
                    .or_insert_with(|| Arc::new(SingleTypeCache::new()))
                    .clone()
            } else {
                typed_caches.get(&type_id).unwrap().clone()
            }
        };

        let create_fn = RefCell::new(Some(create_fn));
        let create_fn_obj: &dyn Fn() -> Arc<dyn Any + Send + Sync + 'static> = &|| {
            let create_fn = create_fn.borrow_mut().take().unwrap();
            let payload = create_fn();
            Arc::new(payload)
        };

        Arc::downcast(typed_cache_obj.get_or_insert_with(identity, create_fn_obj)).unwrap()
    }
}
