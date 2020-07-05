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
    fn shared(self) -> Arc<Lazy<T>> {
        Arc::new(self)
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

trait EvalLazy<T: LazyReqs> {
    type Output;
    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + Sync + 'static>>;
}

impl<T: LazyReqs> EvalLazy<T> for Lazy<T> {
    type Output = T;

    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + Sync + 'static>> {
        let worker = self.worker;
        let cache = cache.clone();

        Box::pin(async move {
            let worker = worker.run(cache);
            let res: T = tokio::task::spawn(worker).await??;
            Ok(res)
        })
    }
}

impl<T: LazyReqs + Sync> EvalLazy<T> for Arc<Lazy<T>> {
    type Output = Arc<T>;
    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + Sync + 'static>> {
        let worker = self.worker.clone_boxed();
        let db = cache.0.clone();

        let cached_val = {
            let db_vals = db.values.read().unwrap();
            db_vals.get(&self.identity).cloned()
        };

        let cache = cache.clone();
        let identity = self.identity;

        Box::pin(async move {
            if let Some(cached) = cached_val {
                Ok(cached
                    .downcast::<T>()
                    .map_err(|e| anyhow!("Could not downcast cached value: {:?}", e))?)
            } else {
                let worker = worker.run(cache);
                let res: Arc<T> = Arc::new(tokio::task::spawn(worker).await??);
                let mut db_vals = db.values.write().unwrap();
                db_vals.insert(identity, res.clone());
                Ok(res)
            }
        })
    }
}

#[derive(Clone)]
struct Cache(Arc<CacheDb>);

impl Cache {
    fn eval<T: LazyReqs, L: EvalLazy<T>>(
        &self,
        lazy: L,
    ) -> impl Future<Output = Result<L::Output>> {
        lazy.eval(self)
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
        _: Cache,
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
    b: Arc<Lazy<i32>>,
}

impl ToLazy for Add {
    type Output = i32;

    fn lazy(self, _: &Cache) -> Lazy<Self::Output> {
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
            let b = cache.eval(self.b.clone()).await?;
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
    dbg!(runtime.block_on(db.eval(c)).unwrap());

    Ok(())
}
