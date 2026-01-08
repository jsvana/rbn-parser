//! Benchmarks for the RBN spot parser.

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use rbn_parser::parser::{looks_like_spot, parse_spot};

/// Sample spot lines for benchmarking.
const SAMPLE_SPOTS: &[&str] = &[
    "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z",
    "DX de KM3T-2-#:  14100.0  CS3B           CW    24 dB  22 WPM  NCDXF B 2259Z",
    "DX de K9LC-#:    28169.9  VA3XCD/B       CW     9 dB  10 WPM  BEACON  2259Z",
    "DX de W1NT-6-#:  28222.9  N1NSP/B        CW     5 dB  15 WPM  BEACON  2259Z",
    "DX de HB9JCB-#:   3516.9  RA1AFT         CW     9 dB  26 WPM  CQ      2259Z",
    "DX de DJ9IE-#:    7028.0  PT7KM          CW    15 dB  10 WPM  CQ      2259Z",
    "DX de LZ4UX-#:    7018.3  RW1M           CW    13 dB  18 WPM  CQ      2259Z",
    "DX de F8DGY-#:    7018.2  RW1M           CW    23 dB  18 WPM  CQ      2259Z",
];

fn bench_parse_spot(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_spot");

    // Benchmark single spot parsing
    group.throughput(Throughput::Elements(1));
    group.bench_function("single", |b| {
        b.iter(|| parse_spot(black_box(SAMPLE_SPOTS[0])))
    });

    // Benchmark batch parsing
    group.throughput(Throughput::Elements(SAMPLE_SPOTS.len() as u64));
    group.bench_function("batch", |b| {
        b.iter(|| {
            for line in SAMPLE_SPOTS {
                let _ = parse_spot(black_box(line));
            }
        })
    });

    group.finish();
}

fn bench_looks_like_spot(c: &mut Criterion) {
    let mut group = c.benchmark_group("looks_like_spot");

    let valid_spot = SAMPLE_SPOTS[0];
    let invalid_line = "Welcome to the Reverse Beacon Network telnet server";

    group.bench_function("valid_spot", |b| {
        b.iter(|| looks_like_spot(black_box(valid_spot)))
    });

    group.bench_function("invalid_line", |b| {
        b.iter(|| looks_like_spot(black_box(invalid_line)))
    });

    group.finish();
}

fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");

    // Mix of valid spots and non-spot lines
    let mixed_lines: Vec<&str> = vec![
        "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z",
        "Welcome to RBN",
        "DX de KM3T-2-#:  14100.0  CS3B           CW    24 dB  22 WPM  NCDXF B 2259Z",
        "",
        "DX de K9LC-#:    28169.9  VA3XCD/B       CW     9 dB  10 WPM  BEACON  2259Z",
        "Your callsign?",
    ];

    group.throughput(Throughput::Elements(mixed_lines.len() as u64));
    group.bench_function("mixed_input", |b| {
        b.iter(|| {
            for line in &mixed_lines {
                if looks_like_spot(line) {
                    let _ = parse_spot(black_box(line));
                }
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_spot,
    bench_looks_like_spot,
    bench_full_pipeline
);
criterion_main!(benches);
