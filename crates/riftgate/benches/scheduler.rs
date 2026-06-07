//! Microbenchmarks for [`riftgate::scheduler::PerShardScheduler`].

use criterion::{BenchmarkId, Criterion, criterion_main};
use riftgate::scheduler::PerShardScheduler;
use riftgate_core::scheduler::Scheduler;

fn bench_scheduler_submit(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler/per_shard_submit");
    for (shards, cap) in [(2_usize, 8_192_usize), (4, 16_384)] {
        group.bench_with_input(
            BenchmarkId::new("submit_noop", format!("{shards}shards_{cap}cap")),
            &(shards, cap),
            |b, &(shards, cap)| {
                let sched = PerShardScheduler::start(shards, cap);
                b.iter(|| {
                    sched.submit(Box::new(|| {}));
                });
                sched.shutdown();
            },
        );
    }
    group.finish();
}

mod harness {
    use super::bench_scheduler_submit;
    use criterion::criterion_group;
    criterion_group!(scheduler, bench_scheduler_submit);
}
criterion_main!(harness::scheduler);
