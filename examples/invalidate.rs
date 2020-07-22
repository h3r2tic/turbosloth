use tokio::runtime::Runtime;
use turbosloth::*;

#[derive(Clone, Hash, IntoLazy)]
struct Frobnicate;

#[async_trait]
impl LazyWorker for Frobnicate {
    type Output = String;

    async fn run(self, _: RunContext) -> Result<Self::Output> {
        println!("Frobnicating");
        Ok("frob".to_owned())
    }
}

fn main() -> Result<()> {
    let cache = Cache::create();
    let mut runtime = Runtime::new()?;

    let a = Frobnicate.into_lazy();
    dbg!(runtime.block_on(a.eval(&cache))?);
    dbg!(runtime.block_on(a.eval(&cache))?);
    println!("Invalidating the lazy reference.");
    a.invalidate();
    dbg!(runtime.block_on(a.eval(&cache))?);

    Ok(())
}
