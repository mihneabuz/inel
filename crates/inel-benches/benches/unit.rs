use criterion::{Criterion, criterion_group, criterion_main};
use std::{hint::black_box, iter::repeat_with};

fn spawn(count: u32) {
    for _ in 0..count {
        inel::spawn(async { black_box(1) + black_box(2) });
    }

    inel::run();
}

fn spawn_bench(c: &mut Criterion) {
    for count in [1_000, 10_000, 100_000] {
        c.bench_function(&format!("spawn {count}"), |b| {
            b.iter(|| spawn(black_box(count)))
        });
    }
}

fn submit(count: u32) {
    inel::block_on(async move {
        for _ in 0..count {
            inel::time::Instant::new().await;
        }
    })
}

fn submit_bench(c: &mut Criterion) {
    for count in [1_000, 10_000, 100_000] {
        c.bench_function(&format!("submit {count}"), |b| {
            b.iter(|| submit(black_box(count)))
        });
    }
}

fn entries(count: u32) {
    inel::block_on(async move {
        let _ = futures::future::join_all(
            repeat_with(|| inel::time::Instant::new()).take(count as usize),
        )
        .await;
    })
}

fn entries_bench(c: &mut Criterion) {
    for count in [1_000, 10_000, 32_700] {
        inel::init(inel::RingOptions::default().submissions(count));
        c.bench_function(&format!("entries {count}"), |b| {
            b.iter(|| entries(black_box(count)))
        });
    }
}

criterion_group!(benches, spawn_bench, submit_bench, entries_bench);
criterion_main!(benches);
