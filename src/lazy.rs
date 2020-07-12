use crate::cache::*;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::marker::PhantomData;
use std::{any::Any, future::Future, pin::Pin, sync::Arc};

pub trait LazyReqs: Any + Sized + Send + 'static {}
impl<T: Any + Sized + Send + 'static> LazyReqs for T {}

#[async_trait]
pub trait LazyWorker: Send + Sync + 'static {
    type Output: LazyReqs;

    async fn run(self, cache: Cache) -> Result<Self::Output>;
}

pub trait LazyWorkerEx<T: LazyReqs> {
    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj<T>>;

    fn run_boxed(
        self: Box<Self>,
        cache: Cache,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>;
}

impl<T: LazyReqs, W> LazyWorkerEx<T> for W
where
    W: LazyWorker<Output = T> + Clone + Sized,
{
    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj<T>> {
        Box::new((*self).clone())
    }

    fn run_boxed(
        self: Box<Self>,
        cache: Cache,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>> {
        (*self).run(cache)
    }
}

pub trait LazyWorkerObj<T: LazyReqs>: LazyWorker<Output = T> + LazyWorkerEx<T> {}
impl<T: LazyReqs, W> LazyWorkerObj<T> for W where W: LazyWorker<Output = T> + LazyWorkerEx<T> {}

pub struct Lazy<T: LazyReqs> {
    //worker: Pin<Box<dyn Future<Output = Result<T>> + Send + Sync + 'static>>,
    pub worker: Box<dyn LazyWorkerObj<T>>,
    pub identity: u64,
}

impl<T: LazyReqs + Sync> Lazy<T> {
    pub fn shared(self) -> Arc<Lazy<T>> {
        Arc::new(self)
    }

    /*pub fn rebind<RetVal: ToLazy>(&mut self, _rebind_fn: impl Fn(Self) -> RetVal) {
        todo!();
    }*/
}

impl<T: LazyReqs> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self {
            worker: self.worker.clone_boxed(),
            identity: self.identity,
        }
    }
}

pub trait LazyFeedback<T: LazyReqs> {
    fn feedback(self) -> (Arc<Lazy<T>>, LazyFeedbackTarget<T>);
}

impl<T: LazyReqs + Sync> LazyFeedback<T> for Lazy<T> {
    fn feedback(self) -> (Arc<Lazy<T>>, LazyFeedbackTarget<T>) {
        todo!()
    }
}

pub struct LazyFeedbackTarget<T: LazyReqs> {
    inner: Lazy<T>,
}

impl<T: LazyReqs + Sync> LazyFeedbackTarget<T> {
    pub fn rebind<RetVal: ToLazy>(self, _rebind_fn: impl Fn(Lazy<T>) -> RetVal) -> Lazy<T> {
        //todo!();
        // TODO
        self.inner
    }
}

pub trait EvalLazy<T: LazyReqs> {
    type Output;
    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>>;
}

// Could be used for getting exclusive access to a non-shared Lazy<T>.
// By denying `Send`, the mutable variable needs to be accessed last in the worker,
// Thus making it more difficult to mutate it, or trying to lock it before other
// variables have been evaluated. If they used `prev` values of the variable
// that's about to be mutated, those will get the correct value.
//
// Not really enforceable, since T could be Arc<Rwlock<_>>, and the user
// can still shoot themselves in the foot with `.write()`, and enjoy deadlocks.
#[derive(Debug)]
pub struct NoSend<T>(T, PhantomData<*const ()>);

impl<T> std::ops::Deref for NoSend<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for NoSend<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: LazyReqs> EvalLazy<T> for Lazy<T> {
    type Output = NoSend<T>;

    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>> {
        let worker = self.worker;
        let cache = cache.clone();

        Box::pin(async move {
            let worker = worker.run_boxed(cache);
            let res: T = tokio::task::spawn(worker).await??;
            Ok(NoSend(res, PhantomData))
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
                let worker = worker.run_boxed(cache);
                let res: Arc<T> = Arc::new(tokio::task::spawn(worker).await??);
                let mut db_vals = db.values.write().unwrap();
                db_vals.insert(identity, res.clone());
                Ok(res)
            }
        })
    }
}

pub trait ToLazy
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
