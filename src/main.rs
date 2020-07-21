//#![feature(associated_type_defaults)]
//#![allow(unused_imports)]

#[allow(unused_imports)]
use anyhow::{anyhow, Result};

use async_trait::async_trait;
use std::sync::Arc;
use tokio::runtime::Runtime;

mod cache;
mod lazy;

use cache::*;
use lazy::*;

impl ToLazy for i32 {
    fn identity(&self) -> u64 {
        *self as u64 // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for i32 {
    type Output = i32;

    async fn run(self, _: Arc<Cache>) -> Result<Self::Output> {
        Ok(self)
    }
}

#[derive(Clone)]
struct Add {
    a: Lazy<i32>,
    b: Lazy<i32>,
}

impl ToLazy for Add {
    fn identity(&self) -> u64 {
        self.a.identity * 12345 + self.b.identity // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for Add {
    type Output = i32;

    async fn run(self, _cache: Arc<Cache>) -> Result<Self::Output> {
        let a = self.a.eval().await?;
        let b = self.b.eval().await?;
        println!("running Add({}, {})", *a, *b);
        Ok(*a + *b)
    }
}

fn try_main() -> Result<()> {
    let cache = Cache::create();

    let a = 1i32.lazy(&cache);
    let b = 2i32.lazy(&cache);
    let c = Add { a, b }.lazy(&cache);

    let mut runtime = Runtime::new()?;

    dbg!(*runtime.block_on(c.eval())?);
    dbg!(*runtime.block_on(c.eval())?);

    Ok(())
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("ERROR: {:?}", err);
        err.chain()
            .skip(1)
            .for_each(|cause| eprintln!("because: {}", cause));
        std::process::exit(1);
    }
}
