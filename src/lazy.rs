use crate::cache::*;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{
    any::Any,
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

pub trait LazyReqs: Any + Sized + Send + 'static {}
impl<T: Any + Sized + Send + 'static> LazyReqs for T {}

#[async_trait]
pub trait LazyWorker: Send + Sync + 'static {
    type Output: LazyReqs;

    async fn run(self, cache: Arc<Cache>) -> Result<Self::Output>;
}

pub trait LazyWorkerEx<T: LazyReqs> {
    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj<T>>;

    fn run_boxed(
        self: Box<Self>,
        cache: Arc<Cache>,
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
        cache: Arc<Cache>,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>> {
        (*self).run(cache)
    }
}

pub trait LazyWorkerObj<T: LazyReqs>: LazyWorker<Output = T> + LazyWorkerEx<T> {}
impl<T: LazyReqs, W> LazyWorkerObj<T> for W where W: LazyWorker<Output = T> + LazyWorkerEx<T> {}

pub struct LazyPayload<T: LazyReqs> {
    pub worker: Box<dyn LazyWorkerObj<T>>,
    pub build_result: Arc<BuildResult<T>>,
}

impl<T: LazyReqs> Clone for LazyPayload<T> {
    fn clone(&self) -> Self {
        Self {
            worker: self.worker.clone_boxed().into(),
            build_result: Default::default(),
        }
    }
}

pub struct BuildResult<T: LazyReqs> {
    value: RwLock<Option<Arc<T>>>,
}

impl<T: LazyReqs> Default for BuildResult<T> {
    fn default() -> Self {
        Self {
            value: Default::default(),
        }
    }
}

impl<T: LazyReqs> BuildResult<T> {
    fn is_none(&self) -> bool {
        self.value.read().unwrap().is_none()
    }
}

pub struct Lazy<T: LazyReqs> {
    pub payload: Arc<LazyPayload<T>>,
    pub identity: u64,
    cache: Arc<Cache>,
}

impl<T: LazyReqs> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self {
            payload: self.payload.clone(),
            identity: self.identity,
            cache: self.cache.clone(),
        }
    }
}

pub trait EvalLazy<T: LazyReqs> {
    type Output;
    fn eval(&self) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>>;
}

impl<T: LazyReqs + Sync> EvalLazy<T> for Lazy<T> {
    type Output = Arc<T>;

    fn eval(&self) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>> {
        // HACK; TODO: check if build result doesn't exist or is stale
        if self.payload.build_result.is_none() {
            let worker = self.payload.worker.clone_boxed();
            let build_result = self.payload.build_result.clone();
            let cache = self.cache.clone();

            Box::pin(async move {
                let worker = worker.run_boxed(cache);
                let res: T = tokio::task::spawn(worker).await??;
                *build_result.value.write().unwrap() = Some(Arc::new(res));

                let v: Option<Arc<T>> = build_result.value.read().unwrap().clone();
                if let Some(v) = v {
                    Ok(v)
                } else {
                    Err(anyhow!("The requested asset failed to build"))
                }
            })
        } else {
            let v: Self::Output =
                Option::<&Arc<_>>::cloned(self.payload.build_result.value.read().unwrap().as_ref())
                    .expect("a valid build result");
            Box::pin(async move { Ok(v) })
        }
    }
}

pub trait ToLazy
where
    Self: LazyWorker + Sized + Clone,
{
    fn lazy(self, cache: &Arc<Cache>) -> Lazy<<Self as LazyWorker>::Output> {
        // TODO: find identical entry in the cache

        Lazy {
            identity: self.identity(),
            payload: Arc::new(LazyPayload {
                worker: Box::new(self),
                build_result: Default::default(),
            }),
            cache: cache.clone(),
        }
    }

    fn identity(&self) -> u64;
}
