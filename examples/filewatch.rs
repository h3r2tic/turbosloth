use anyhow::Context as _;
use hotwatch::Hotwatch;
use lazy_static::lazy_static;
use std::{path::PathBuf, sync::Mutex};
use tokio::runtime::Runtime;
use turbosloth::*;

lazy_static! {
    static ref FILE_WATCHER: Mutex<Hotwatch> = Mutex::new(Hotwatch::new().unwrap());
}

#[derive(Clone, Hash, IntoLazy)]
struct CountLinesInFile {
    path: PathBuf,
}

#[async_trait]
impl LazyWorker for CountLinesInFile {
    type Output = usize;
    type Error = anyhow::Error;

    async fn run(self, ctx: RunContext) -> anyhow::Result<Self::Output> {
        let invalidation_trigger = ctx.get_invalidation_trigger();

        FILE_WATCHER
            .lock()
            .unwrap()
            .watch(self.path.clone(), move |_| {
                invalidation_trigger();
            })
            .with_context(|| format!("Trying to watch {:?}", self.path))?;

        println!("Counting lines in file");
        std::fs::read_to_string(&self.path)
            .map(|contents| contents.lines().count())
            .map_err(|err| anyhow::anyhow!("IO error: {:?}", err))
    }
}

fn main() -> anyhow::Result<()> {
    let cache = LazyCache::create();
    let mut runtime = Runtime::new()?;

    let self_line_count = CountLinesInFile {
        path: PathBuf::from("examples/filewatch.rs"),
    }
    .into_lazy();

    loop {
        if !self_line_count.is_up_to_date() {
            dbg!(runtime.block_on(self_line_count.eval(&cache))?);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
