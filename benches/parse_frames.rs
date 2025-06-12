use bwl_meta::{parse_frames, remove_inconsistent};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_remove_inconsistent(c: &mut Criterion) {
    c.bench_function("remove_inconsistent", |b| {
        b.iter(|| {
            let m = parse_frames().unwrap();
            remove_inconsistent(m).unwrap();
        })
    });
}

criterion_group!(benches, bench_remove_inconsistent);
criterion_main!(benches);
