//! Microbenchmarks for [`riftgate_parser::Http1Parser`].
//!
//! Measures the cost of parsing a representative
//! `POST /v1/chat/completions` request as a single feed and as a
//! byte-by-byte split feed (the worst-case path that exercises every
//! state-machine transition).
//!
//! Used as a baseline for v0.2 (zero-copy header buffer) and v0.3
//! (chunked encoding) parser rework.

use criterion::{Criterion, Throughput, criterion_main};
use riftgate_core::parser::{ParseEvent, StreamParser};
use riftgate_parser::Http1Parser;
use std::hint::black_box;

const SAMPLE_REQUEST: &[u8] = b"POST /v1/chat/completions HTTP/1.1\r\n\
                                  Host: api.openai.com\r\n\
                                  Content-Type: application/json\r\n\
                                  Authorization: Bearer sk-test\r\n\
                                  X-Trace-Id: abcdef0123456789\r\n\
                                  Content-Length: 89\r\n\
                                  \r\n\
                                  {\"model\":\"gpt-4o-mini\",\"messages\":[{\"role\":\"user\",\"content\":\"hello world!\"}]}";

fn bench_full_feed(c: &mut Criterion) {
    let mut group = c.benchmark_group("http1/full_feed");
    group.throughput(Throughput::Bytes(SAMPLE_REQUEST.len() as u64));
    group.bench_function("post_chat_completions", |b| {
        b.iter(|| {
            let mut p = Http1Parser::new();
            for ev in p.feed(black_box(SAMPLE_REQUEST)) {
                if let ParseEvent::Error(e) = ev {
                    panic!("unexpected error: {e:?}");
                }
            }
        });
    });
    group.finish();
}

fn bench_byte_by_byte_feed(c: &mut Criterion) {
    let mut group = c.benchmark_group("http1/byte_by_byte_feed");
    group.throughput(Throughput::Bytes(SAMPLE_REQUEST.len() as u64));
    group.bench_function("post_chat_completions", |b| {
        b.iter(|| {
            let mut p = Http1Parser::new();
            for byte in SAMPLE_REQUEST.iter() {
                for ev in p.feed(std::slice::from_ref(byte)) {
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
    use super::{bench_byte_by_byte_feed, bench_full_feed};
    use criterion::criterion_group;
    criterion_group!(http1, bench_full_feed, bench_byte_by_byte_feed);
}
criterion_main!(harness::http1);
