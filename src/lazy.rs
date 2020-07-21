use crate::cache::*;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{
    any::Any,
    future::Future,
    hash::{Hash, Hasher},
    pin::Pin,
    sync::{Arc, Mutex, RwLock},
};

pub use turbosloth_macros::IntoLazy;

pub trait LazyReqs: Any + Sized + Send + Sync + 'static {}
impl<T: Any + Sized + Send + Sync + 'static> LazyReqs for T {}

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

enum LazyInner<T: LazyReqs> {
    Cached(Arc<LazyPayload<T>>),
    Isolated(Arc<dyn LazyWorkerObj<T>>),
}

impl<T: LazyReqs> Clone for LazyInner<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Cached(cached) => Self::Cached(cached.clone()),
            Self::Isolated(isolated) => Self::Isolated(isolated.clone()),
        }
    }
}

pub struct Lazy<T: LazyReqs> {
    inner: Mutex<LazyInner<T>>,
    pub identity: u64,
}

impl<T: LazyReqs> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Mutex::new(self.inner.lock().unwrap().clone()),
            identity: self.identity,
        }
    }
}

impl<T: LazyReqs> Hash for Lazy<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
    }
}

pub trait EvalLazy<T: LazyReqs> {
    fn eval(
        &self,
        cache: &Arc<Cache>,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<T>>> + Send + 'static>>;
}

impl<T: LazyReqs> EvalLazy<T> for Lazy<T> {
    fn eval(
        &self,
        cache: &Arc<Cache>,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<T>>> + Send + 'static>> {
        let payload = {
            let mut inner = self.inner.lock().unwrap();

            match &mut *inner {
                LazyInner::Cached(cached) => cached.clone(),
                LazyInner::Isolated(isolated) => {
                    let worker = isolated.clone_boxed();
                    let cached = cache.get_or_insert_with(self.identity, move || LazyPayload {
                        worker,
                        build_result: Default::default(),
                    });

                    let result = cached.clone();

                    // Connect to cache, and return the cached payload
                    *inner = LazyInner::Cached(cached);
                    result
                }
            }
        };

        // HACK; TODO: check if build result doesn't exist or is stale
        if payload.build_result.is_none() {
            let worker = payload.worker.clone_boxed();
            let build_result = payload.build_result.clone();
            let cache = cache.clone();

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
            let v: Arc<T> =
                Option::<&Arc<_>>::cloned(payload.build_result.value.read().unwrap().as_ref())
                    .expect("a valid build result");
            Box::pin(async move { Ok(v) })
        }
    }
}

pub trait ToLazyIdentity {
    fn identity(&self) -> u64;
}

impl<T: Hash> ToLazyIdentity for T {
    fn identity(&self) -> u64 {
        let mut s = twox_hash::XxHash64::default();
        <Self as std::hash::Hash>::hash(&self, &mut s);
        s.finish()
    }
}

pub trait IntoLazy: ToLazyIdentity
where
    Self: LazyWorker + Sized + Clone,
{
    fn into_lazy(self) -> Lazy<<Self as LazyWorker>::Output> {
        let identity = self.identity();

        Lazy {
            identity: identity,
            inner: Mutex::new(LazyInner::Isolated(Arc::new(self))),
        }
    }
}
