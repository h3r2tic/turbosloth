use tokio::runtime::Runtime;
use turbosloth::*;

static mut REFORBLE: Option<Box<dyn Fn() + Send + Sync>> = None;

#[derive(Clone, Hash, IntoLazy)]
struct Forble;

#[async_trait]
impl LazyWorker for Forble {
    type Output = String;

    async fn run(self, ctx: RunContext) -> Result<Self::Output> {
        unsafe {
            REFORBLE = Some(Box::new(ctx.get_invalidation_trigger()));
        }

        println!("Forbling");
        Ok("forble".to_owned())
    }
}

#[derive(Clone, Hash, IntoLazy)]
struct Borble {
    forble: Lazy<String>,
}

#[async_trait]
impl LazyWorker for Borble {
    type Output = String;

    async fn run(self, ctx: RunContext) -> Result<Self::Output> {
        println!("Borbling the forble");
        Ok((*self.forble.eval(ctx).await?).clone() + "borble")
    }
}

fn main() -> Result<()> {
    let cache = Cache::create();
    let mut runtime = Runtime::new()?;

    let boop = Borble {
        forble: Forble.into_lazy(),
    }
    .into_lazy();
    dbg!(runtime.block_on(boop.eval(&cache))?);
    dbg!(runtime.block_on(boop.eval(&cache))?);

    println!("Invalidating the forble!");
    unsafe {
        (REFORBLE.as_ref().unwrap())();
    }

    dbg!(runtime.block_on(boop.eval(&cache))?);

    Ok(())
}
