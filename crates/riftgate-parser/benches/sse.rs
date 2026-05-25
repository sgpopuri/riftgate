//! Microbenchmarks for [`riftgate_parser::SseFramer`].
//!
//! Drives a 64-event SSE stream through the framer in three modes:
//! one chunk, ten chunks (typical TCP segmentation), and byte-by-byte
//! (worst-case state-machine churn).

use criterion::{Criterion, Throughput, criterion_main};
use riftgate_core::parser::{ParseEvent, StreamParser};
use riftgate_parser::SseFramer;

fn build_stream(events: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(events * 64);
    for i in 0..events {
        out.extend_from_slice(b"data: {\"choices\":[{\"delta\":{\"content\":\"token");
        out.extend_from_slice(format!("{i:04}").as_bytes());
        out.extend_from_slice(b"\"}}]}\n\n");
    }
    out.extend_from_slice(b"data: [DONE]\n\n");
    out
}

fn bench_single_feed(c: &mut Criterion) {
    let stream = build_stream(64);
    let mut group = c.benchmark_group("sse/single_feed");
    group.throughput(Throughput::Bytes(stream.len() as u64));
    group.bench_function("64_events", |b| {
        b.iter(|| {
            let mut f = SseFramer::new();
            for ev in f.feed(&stream) {
                if let ParseEvent::Error(e) = ev {
                    panic!("unexpected error: {e:?}");
                }
            }
        });
    });
    group.finish();
}

fn bench_ten_chunk_feed(c: &mut Criterion) {
    let stream = build_stream(64);
    let chunk_size = (stream.len() / 10).max(1);
    let mut group = c.benchmark_group("sse/ten_chunk_feed");
    group.throughput(Throughput::Bytes(stream.len() as u64));
    group.bench_function("64_events", |b| {
        b.iter(|| {
            let mut f = SseFramer::new();
            for chunk in stream.chunks(chunk_size) {
                for ev in f.feed(chunk) {
                    if let ParseEvent::Error(e) = ev {
                        panic!("unexpected error: {e:?}");
                    }
                }
            }
        });
    });
    group.finish();
}

fn bench_byte_by_byte_feed(c: &mut Criterion) {
    let stream = build_stream(16);
    let mut group = c.benchmark_group("sse/byte_by_byte_feed");
    group.throughput(Throughput::Bytes(stream.len() as u64));
    group.bench_function("16_events", |b| {
        b.iter(|| {
            let mut f = SseFramer::new();
            for byte in stream.iter() {
                for ev in f.feed(std::slice::from_ref(byte)) {
                    if let ParseEvent::Error(e) = ev {
                        panic!("unexpected error: {e:?}");
                    }
                }
            }
        });
    });
    group.finish();
}

mod harness {
    use super::{bench_byte_by_byte_feed, bench_single_feed, bench_ten_chunk_feed};
    use criterion::criterion_group;
    criterion_group!(
        sse,
        bench_single_feed,
        bench_ten_chunk_feed,
        bench_byte_by_byte_feed
    );
}
criterion_main!(harness::sse);
