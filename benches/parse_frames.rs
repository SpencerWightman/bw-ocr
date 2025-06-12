use std::fs;

use bwl_meta::{FRAMES_DIR, parse_frames, remove_inconsistent};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_remove_inconsistent(c: &mut Criterion) {
    c.bench_function("remove_inconsistent", |b| {
        b.iter(|| {
            let seg_vec_len = fs::read_dir(FRAMES_DIR)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .count();
            let seg_vec = Vec::with_capacity(seg_vec_len);
            let populated_seg_vec = parse_frames(seg_vec).unwrap();
            remove_inconsistent(populated_seg_vec).unwrap();
        })
    });
}

criterion_group!(benches, bench_remove_inconsistent);
criterion_main!(benches);
