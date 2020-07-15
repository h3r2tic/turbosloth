#![allow(unused_imports)]

use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, RwLock},
};

pub struct CacheDb {
    //pub(crate) values: RwLock<HashMap<u64, Arc<dyn Any + Send + Sync>>>,
}

impl CacheDb {
    pub fn create() -> Cache {
        Cache(Arc::new(Self {
            //values: RwLock::new(Default::default()),
        }))
    }
}

#[derive(Clone)]
pub struct Cache(pub Arc<CacheDb>);
/*
impl Cache {
    fn eval<T: LazyReqs, L: EvalLazy<T>>(
        &self,
        lazy: L,
    ) -> impl Future<Output = Result<L::Output>> {
        lazy.eval(self)
    }
}*/
