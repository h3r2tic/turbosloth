#![feature(associated_type_defaults)]
#![allow(unused_imports)]

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{
    any::Any,
    collections::HashMap,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, RwLock},
};
use tokio::runtime::Runtime;

trait LazyReqs: Any + Sized + Send + 'static {}
impl<T: Any + Sized + Send + 'static> LazyReqs for T {}

trait LazyWorker<T: LazyReqs>: Send + Sync {
    fn run(
        self: Box<Self>,
        cache: Cache,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + Sync + 'static>>;
    fn clone_boxed(&self) -> Box<dyn LazyWorker<T>>;
}

struct Lazy<T: LazyReqs> {
    //worker: Pin<Box<dyn Future<Output = Result<T>> + Send + Sync + 'static>>,
    worker: Box<dyn LazyWorker<T>>,
    identity: u64,
}

impl<T: LazyReqs + Sync> Lazy<T> {
    fn shared(self) -> SharedLazy<T> {
        SharedLazy {
            worker: Arc::new(self.worker),
            identity: self.identity,
        }
    }
}

impl<T: LazyReqs> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self {
            worker: self.worker.clone_boxed(),
            identity: self.identity,
        }
    }
}

#[derive(Clone)]
struct SharedLazy<T: LazyReqs> {
    worker: Arc<Box<dyn LazyWorker<T>>>,
    identity: u64,
}

impl<T: LazyReqs> SharedLazy<T> {
    fn cloned(&self) -> Self {
        Self {
            worker: self.worker.clone(),
            identity: self.identity,
        }
    }
}

struct CacheDb {
    values: RwLock<HashMap<u64, Arc<dyn Any + Send + Sync>>>,
}

impl CacheDb {
    fn create() -> Cache {
        Cache(Arc::new(Self {
            values: RwLock::new(Default::default()),
        }))
    }
}

#[derive(Clone)]
struct Cache(Arc<CacheDb>);

impl Cache {
    fn eval<T: LazyReqs>(&self, lazy: Lazy<T>) -> impl Future<Output = Result<T>> {
        let worker = lazy.worker;
        let cache = self.clone();
        async move {
            let worker = worker.run(cache);
            let res: T = tokio::task::spawn(worker).await??;
            Ok(res)
        }
    }

    fn eval_shared<T: LazyReqs + Sync>(
        &self,
        lazy: SharedLazy<T>,
    ) -> impl Future<Output = Result<Arc<T>>> {
        let worker = lazy.worker.clone_boxed();
        let db = self.0.clone();

        let cached_val = {
            let db_vals = db.values.read().unwrap();
            db_vals.get(&lazy.identity).cloned()
        };

        let cache = self.clone();

        async move {
            if let Some(cached) = cached_val {
                Ok(cached
                    .downcast::<T>()
                    .map_err(|e| anyhow!("Could not downcast cached value"))?)
            } else {
                let worker = worker.run(cache);
                let res: Arc<T> = Arc::new(tokio::task::spawn(worker).await??);
                let mut db_vals = db.values.write().unwrap();
                db_vals.insert(lazy.identity, res.clone());
                Ok(res)
            }
        }
    }
}

trait ToLazy
where
    Self: Sized,
{
    type Output: LazyReqs;
    fn lazy(self, db: &Cache) -> Lazy<Self::Output>;
}

// ----

impl ToLazy for i32 {
    type Output = i32;

    fn lazy(self, _cache: &Cache) -> Lazy<Self::Output> {
        Lazy {
            worker: Box::new(self),
            identity: self as u64, // TODO: hash
        }
    }
}

impl LazyWorker<i32> for i32 {
    fn run(
        self: Box<Self>,
        cache: Cache,
    ) -> Pin<Box<dyn Future<Output = Result<i32>> + Send + Sync + 'static>> {
        Box::pin(async move { Ok(*self) })
    }

    fn clone_boxed(&self) -> Box<dyn LazyWorker<i32>> {
        Box::new(*self)
    }
}

#[derive(Clone)]
struct Add {
    a: Lazy<i32>,
    b: SharedLazy<i32>,
}

impl ToLazy for Add {
    type Output = i32;

    fn lazy(self, cache: &Cache) -> Lazy<Self::Output> {
        let cache = cache.clone();
        Lazy {
            identity: self.a.identity * 12345 + self.b.identity, // TODO: hash
            worker: Box::new(self),
        }
    }
}

impl LazyWorker<i32> for Add {
    fn run(
        self: Box<Self>,
        cache: Cache,
    ) -> Pin<Box<dyn Future<Output = Result<i32>> + Send + Sync + 'static>> {
        Box::pin(async move {
            let a = cache.eval(self.a).await?;
            let b = cache.eval_shared(self.b.cloned()).await?;
            println!("running Add({}, {})", a, *b);
            Ok(a + *b)
        })
    }

    fn clone_boxed(&self) -> Box<dyn LazyWorker<i32>> {
        Box::new(self.clone())
    }
}

fn main() -> Result<()> {
    let db = CacheDb::create();

    let a = 1i32.lazy(&db);
    let b = 2i32.lazy(&db).shared();
    let c = Add { a: a.clone(), b }.lazy(&db).shared();
    let d = Add { a, b: c.clone() }.lazy(&db);

    let mut runtime = Runtime::new()?;
    dbg!(runtime.block_on(db.eval(d.clone())).unwrap());
    dbg!(runtime.block_on(db.eval(d)).unwrap());
    dbg!(runtime.block_on(db.eval_shared(c)).unwrap());

    Ok(())
}
