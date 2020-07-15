use crate::cache::*;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::marker::PhantomData;
use std::{
    any::Any,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, RwLock},
};

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

pub struct LazyOwnedPayload<T: LazyReqs> {
    pub worker: Box<dyn LazyWorkerObj<T>>,
    pub build_result: Arc<BuildResult<T>>,
}

impl<T: LazyReqs> Clone for LazyOwnedPayload<T> {
    fn clone(&self) -> Self {
        Self {
            worker: self.worker.clone_boxed().into(),
            build_result: Default::default(),
        }
    }
}

pub struct BuildResult<T: LazyReqs> {
    value: RwLock<Arc<Option<T>>>,
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

    fn is_some(&self) -> bool {
        self.value.read().unwrap().is_some()
    }
}

pub struct LazyPrevPayload<T: LazyReqs> {
    pub prev_worker: Mutex<Option<Box<dyn LazyWorkerObj<T>>>>,
    pub build_result: Arc<BuildResult<T>>,
}

impl<T: LazyReqs> Clone for LazyPrevPayload<T> {
    fn clone(&self) -> Self {
        Self {
            prev_worker: Mutex::new(None),
            build_result: Default::default(),
        }
    }
}

pub enum LazyValuePayload<T: LazyReqs> {
    Actual(LazyOwnedPayload<T>),
    Prev(LazyPrevPayload<T>),
}

impl<T: LazyReqs> Clone for LazyValuePayload<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Actual(v) => Self::Actual(v.clone()),
            Self::Prev(v) => Self::Prev(v.clone()),
        }
    }
}

pub struct Lazy<T: LazyReqs> {
    pub payload: LazyValuePayload<T>,
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
            payload: self.payload.clone(),
            identity: self.identity,
        }
    }
}

pub trait LazyFeedback<T: LazyReqs> {
    fn feedback(self) -> (Arc<Lazy<T>>, LazyFeedbackTarget<T>);
}

impl<T: LazyReqs + Sync> LazyFeedback<T> for Lazy<T> {
    fn feedback(self) -> (Arc<Lazy<T>>, LazyFeedbackTarget<T>) {
        if let LazyValuePayload::Actual(payload) = self.payload {
            let prev = Arc::new(Lazy {
                payload: LazyValuePayload::Prev(LazyPrevPayload {
                    build_result: payload.build_result.clone(),
                    prev_worker: Mutex::new(Some(payload.worker)),
                }),
                identity: self.identity,
            });

            let actual = LazyFeedbackTarget {
                identity: self.identity,
                prev: Lazy {
                    payload: LazyValuePayload::Prev(LazyPrevPayload {
                        build_result: payload.build_result,
                        prev_worker: Mutex::new(None),
                    }),
                    identity: self.identity,
                },
            };

            (prev, actual)
        } else {
            panic!("feedback() can only be called on owned lazy references")
        }
    }
}

pub struct LazyFeedbackTarget<T: LazyReqs> {
    identity: u64,
    prev: Lazy<T>,
}

impl<T: LazyReqs + Sync> LazyFeedbackTarget<T> {
    pub fn rebind<RetVal>(self, rebind_fn: impl Fn(Lazy<T>) -> RetVal) -> Lazy<T>
    where
        RetVal: ToLazy,
        RetVal: LazyWorker<Output = ()>,
    {
        //todo!();
        // TODO
        rebind_fn(self.prev).lazy()
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

// TODO: validate at runtime that a shared ref does not exist for this item
impl<T: LazyReqs> EvalLazy<T> for Lazy<T> {
    type Output = NoSend<T>;

    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>> {
        /*let payload = Arc::try_unwrap(self.payload)
        .ok()
        .expect("Lazy::eval: more than one reference exists!");*/

        let payload = if let LazyValuePayload::Actual(payload) = self.payload {
            payload
        } else {
            panic!("An owned Lazy value must contain LazyValuePayload::Actual")
        };

        let worker = payload.worker;
        /*let worker = Arc::into_raw(payload.worker);
        let worker: Box<dyn LazyWorkerObj<T>> = unsafe { Box::from_raw(worker as *mut _) };*/

        let cache = cache.clone();

        Box::pin(async move {
            let worker = worker.run_boxed(cache);
            let res: T = tokio::task::spawn(worker).await??;
            Ok(NoSend(res, PhantomData))
        })
    }
}

pub struct LazyValueReadRef<T: LazyReqs>(Arc<Option<T>>);
pub struct LazyValueWriteRef<T: LazyReqs>(Arc<T>);

//unsafe impl<T: LazyReqs + Send> Send for LazyValueReadRef<T> {}
impl<T: LazyReqs> std::ops::Deref for LazyValueReadRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        (&*self.0).as_ref().unwrap()
    }
}

impl<T: LazyReqs + Sync> EvalLazy<T> for Arc<Lazy<T>> {
    type Output = LazyValueReadRef<T>;

    fn eval(
        self,
        cache: &Cache,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output>> + Send + 'static>> {
        match &self.payload {
            // Shared ref
            LazyValuePayload::Actual(payload) => {
                // TODO: run worker if necessary

                /*let worker = payload.worker.clone_boxed();
                let db = cache.0.clone();

                let cached_val = {
                    let db_vals = db.values.read().unwrap();
                    db_vals.get(&self.identity).cloned()
                };

                let cache = cache.clone();
                let identity = self.identity;

                Ok(Box::pin(async move {
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
                }))*/

                // HACK; TODO: check if build result doesn't exist or is stale
                if payload.build_result.is_none() {
                    let worker = payload.worker.clone_boxed();
                    let build_result = payload.build_result.clone();
                    let cache = cache.clone();

                    Box::pin(async move {
                        let worker = worker.run_boxed(cache);
                        let res: T = tokio::task::spawn(worker).await??;
                        *Arc::get_mut(&mut build_result.value.write().unwrap()).unwrap() =
                            Some(res);

                        let v: Self::Output =
                            LazyValueReadRef(build_result.value.read().unwrap().clone());

                        Ok(v)
                    })
                } else {
                    if payload.build_result.is_some() {
                        let v: Self::Output =
                            LazyValueReadRef(payload.build_result.value.read().unwrap().clone());

                        Box::pin(async move { Ok(v) })
                    } else {
                        Box::pin(async move { Err(anyhow!("The requested asset failed to build")) })
                    }
                }
            }
            LazyValuePayload::Prev(payload) => {
                if payload.build_result.is_some() {
                    let v: Self::Output =
                        LazyValueReadRef(payload.build_result.value.read().unwrap().clone());

                    Box::pin(async move { Ok(v) })
                } else {
                    Box::pin(async move {
                        Err(anyhow!(
                            "A previous value for a lazy reference is not available."
                        ))
                    })
                }
            }
        }
    }
}

pub trait ToLazy
where
    Self: LazyWorker + Sized + Clone,
{
    fn lazy(self) -> Lazy<<Self as LazyWorker>::Output> {
        Lazy {
            identity: self.identity(),
            payload: LazyValuePayload::Actual(LazyOwnedPayload {
                worker: Box::new(self),
                build_result: Default::default(),
            }),
        }
    }

    fn identity(&self) -> u64;
}
