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

#[async_trait]
trait LazyWorker: Send + Sync + 'static {
    type Output: LazyReqs;

    async fn run(self: Box<Self>, cache: Cache) -> Result<Self::Output>;
}

trait LazyWorkedCloneBoxed<T: LazyReqs> {
    fn clone_boxed(&self) -> Box<dyn LazyWorkedObj<T>>;
}

impl<T: LazyReqs, W> LazyWorkedCloneBoxed<T> for W
where
    W: LazyWorker<Output = T> + Clone,
{
    fn clone_boxed(&self) -> Box<dyn LazyWorkedObj<T>> {
        Box::new((*self).clone())
    }
}

trait LazyWorkedObj<T: LazyReqs>: LazyWorker<Output = T> + LazyWorkedCloneBoxed<T> {}
impl<T: LazyReqs, W> LazyWorkedObj<T> for W where W: LazyWorker<Output = T> + LazyWorkedCloneBoxed<T>
{}

struct Lazy<T: LazyReqs> {
    //worker: Pin<Box<dyn Future<Output = Result<T>> + Send + Sync + 'static>>,
    worker: Box<dyn LazyWorkedObj<T>>,
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
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>>;
}

impl<T: LazyReqs> EvalLazy<T> for Lazy<T> {
    type Output = T;

    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>> {
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
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>> {
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
    Self: LazyWorker + Sized + Clone,
{
    fn lazy(self) -> Lazy<<Self as LazyWorker>::Output> {
        Lazy {
            identity: self.identity(),
            worker: Box::new(self),
        }
    }

    fn identity(&self) -> u64;
}

// ----

impl ToLazy for i32 {
    fn identity(&self) -> u64 {
        *self as u64 // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for i32 {
    type Output = i32;

    async fn run(self: Box<Self>, _: Cache) -> Result<Self::Output> {
        Ok(*self)
    }
}

#[derive(Clone)]
struct Add {
    a: Lazy<i32>,
    b: Arc<Lazy<i32>>,
}

impl ToLazy for Add {
    fn identity(&self) -> u64 {
        self.a.identity * 12345 + self.b.identity // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for Add {
    type Output = i32;

    async fn run(self: Box<Self>, cache: Cache) -> Result<Self::Output> {
        let a = cache.eval(self.a).await?;
        let b = cache.eval(self.b.clone()).await?;
        println!("running Add({}, {})", a, *b);
        Ok(a + *b)
    }
}

fn main() -> Result<()> {
    let a = 1i32.lazy();
    let b = 2i32.lazy().shared();
    let c = Add { a: a.clone(), b }.lazy().shared();
    let d = Add { a, b: c.clone() }.lazy();

    let cache = CacheDb::create();
    let mut runtime = Runtime::new()?;

    dbg!(runtime.block_on(cache.eval(d.clone()))?);
    dbg!(runtime.block_on(cache.eval(d))?);
    dbg!(runtime.block_on(cache.eval(c))?);

    Ok(())
}
