use crate::cache::*;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{
    any::{Any, TypeId},
    future::Future,
    hash::{Hash, Hasher},
    marker::PhantomData,
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

type BoxedWorkerFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn Any + Send + Sync>>> + Send + 'static>>;

pub trait LazyWorkerObj: Send + Sync {
    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj>;
    fn run_boxed(self: Box<Self>, cache: Arc<Cache>) -> BoxedWorkerFuture;
}

impl<T: LazyReqs, W> LazyWorkerObj for W
where
    W: LazyWorker<Output = T> + Clone,
{
    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj> {
        Box::new((*self).clone())
    }

    fn run_boxed(self: Box<Self>, cache: Arc<Cache>) -> BoxedWorkerFuture {
        Box::pin(async {
            (*self)
                .run(cache)
                .await
                .map(|result| -> Arc<dyn Any + Send + Sync> { Arc::new(result) })
        })
    }
}

pub struct LazyPayload {
    pub worker: Box<dyn LazyWorkerObj>,
    pub build_record: RwLock<BuildRecord>,
}

impl Clone for LazyPayload {
    fn clone(&self) -> Self {
        Self {
            worker: self.worker.clone_boxed(),
            build_record: Default::default(),
        }
    }
}

pub struct BuildRecord {
    artifact: Option<Arc<dyn Any + Send + Sync>>,
}

impl Default for BuildRecord {
    fn default() -> Self {
        Self {
            artifact: Default::default(),
        }
    }
}

enum LazyInner {
    Cached(Arc<LazyPayload>),
    Isolated(Arc<dyn LazyWorkerObj>),
}

impl Clone for LazyInner {
    fn clone(&self) -> Self {
        match self {
            Self::Cached(cached) => Self::Cached(cached.clone()),
            Self::Isolated(isolated) => Self::Isolated(isolated.clone()),
        }
    }
}

pub struct Lazy<T: LazyReqs> {
    inner: Mutex<LazyInner>,
    identity: u64,
    pub debug_name: &'static str,
    marker: PhantomData<T>,
}

impl<T: LazyReqs> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Mutex::new(self.inner.lock().unwrap().clone()),
            identity: self.identity,
            debug_name: self.debug_name,
            marker: PhantomData,
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
                    let type_id = TypeId::of::<T>();
                    let cached =
                        cache.get_or_insert_with(type_id, self.identity, move || LazyPayload {
                            worker,
                            build_record: Default::default(),
                        });

                    let result = cached.clone();

                    // Connect to cache, and return the cached payload
                    *inner = LazyInner::Cached(cached);
                    result
                }
            }
        };

        // HACK; TODO: check if build result doesn't exist or is stale
        if payload.build_record.read().unwrap().artifact.is_none() {
            let worker = payload.worker.clone_boxed();
            let cache = cache.clone();

            log::info!("Evaluating {}", self.debug_name);

            Box::pin(async move {
                let worker = worker.run_boxed(cache);
                let res: Arc<dyn Any + Send + Sync> = tokio::task::spawn(worker).await??;

                let mut build_record = payload.build_record.write().unwrap();
                build_record.artifact = Some(res);

                let v: Option<Arc<T>> = build_record
                    .artifact
                    .clone()
                    .map(|artifact| Arc::downcast::<T>(artifact).expect("downcast"));

                if let Some(v) = v {
                    Ok(v)
                } else {
                    Err(anyhow!("The requested asset failed to build"))
                }
            })
        } else {
            let build_record = payload.build_record.read().unwrap();
            let v: Option<Arc<T>> = build_record
                .artifact
                .clone()
                .map(|artifact| Arc::downcast::<T>(artifact).expect("downcast"));
            let v: Arc<T> = v.expect("a valid build result");
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
            identity,
            inner: Mutex::new(LazyInner::Isolated(Arc::new(self))),
            debug_name: std::any::type_name::<Self>(),
            marker: PhantomData,
        }
    }
}
