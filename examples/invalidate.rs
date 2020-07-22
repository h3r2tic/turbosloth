use tokio::runtime::Runtime;
use turbosloth::*;

static mut REFROB: Option<Box<dyn Fn() + Send + Sync>> = None;

#[derive(Clone, Hash, IntoLazy)]
struct Frobnicate;

#[async_trait]
impl LazyWorker for Frobnicate {
    type Output = String;

    async fn run(self, ctx: RunContext) -> Result<Self::Output> {
        unsafe {
            REFROB = Some(Box::new(ctx.get_invalidation_trigger()));
        }

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

    println!("Invalidating the worker result.");
    unsafe {
        (REFROB.as_ref().unwrap())();
    }

    dbg!(runtime.block_on(a.eval(&cache))?);

    Ok(())
}
