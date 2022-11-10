use async_trait::async_trait;
use crate::{Bench, BenchEnv, sim_zone::SimZone};
use log::*;

pub struct NDNBench {}

#[async_trait]
impl Bench for NDNBench {
    async fn bench(&self) -> bool {
        info!("begin test NDNBench...");
        let begin = std::time::Instant::now();

        let ret = if env == BenchEnv::Simulator {
            true
        } else {
            // TODO: support physical stack  ood/runtime
            true
        };

        let dur = begin.elapsed();
        info!("end test NDNBench: {:?}", dur);

        ret
        
    }

    fn name(&self) -> &str {
        "NDN Bench"
    }
}