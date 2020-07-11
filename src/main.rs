//#![feature(associated_type_defaults)]
//#![allow(unused_imports)]

#[allow(unused_imports)]
use anyhow::{anyhow, Result};

use async_trait::async_trait;
use std::sync::Arc;
use tokio::runtime::Runtime;

mod lazy;
use lazy::*;

impl ToLazy for i32 {
    fn identity(&self) -> u64 {
        *self as u64 // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for i32 {
    type Output = i32;

    async fn run(self, _: Cache) -> Result<Self::Output> {
        Ok(self)
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

    async fn run(self, cache: Cache) -> Result<Self::Output> {
        let a = self.a.eval(&cache).await?;
        let b = self.b.clone().eval(&cache).await?;
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

    let (f_prev, mut f) = 0i32.lazy().feedback();
    f.rebind(Add {
        a: f_prev,
        b: 1i32.lazy().shared(),
    });

    dbg!(runtime.block_on(d.clone().eval(&cache))?);
    dbg!(runtime.block_on(d.eval(&cache))?);
    dbg!(runtime.block_on(c.eval(&cache))?);

    dbg!(runtime.block_on(f.clone().eval(&cache))?);
    dbg!(runtime.block_on(f.clone().eval(&cache))?);
    dbg!(runtime.block_on(f.clone().eval(&cache))?);

    Ok(())
}
