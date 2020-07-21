#![allow(unused_imports)]

use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, RwLock},
};

pub struct Cache {
    //pub(crate) values: RwLock<HashMap<u64, Arc<dyn Any + Send + Sync>>>,
}

impl Cache {
    pub fn create() -> Arc<Cache> {
        Arc::new(Self {
            //values: RwLock::new(Default::default()),
        })
    }
}

/*
impl Cache {
    fn eval<T: LazyReqs, L: EvalLazy<T>>(
        &self,
        lazy: L,
    ) -> impl Future<Output = Result<L::Output>> {
        lazy.eval(self)
    }
}*/
