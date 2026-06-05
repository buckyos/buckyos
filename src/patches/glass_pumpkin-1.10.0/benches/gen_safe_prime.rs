use criterion::{Criterion, criterion_group, criterion_main};
use glass_pumpkin::{prime, safe_prime};

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("gen_prime u256", |b| b.iter(|| prime::new(256).unwrap()));
    c.bench_function("gen_safe_prime u256", |b| {
        b.iter(|| safe_prime::new(256).unwrap())
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
