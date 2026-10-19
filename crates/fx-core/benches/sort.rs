use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use fx_core::{Batch, DirSnapshot, SortDir, SortKey, SortSpec};

const N: usize = 100_000;

fn batch(n: usize, offset: usize) -> Batch {
    let mut b = Batch::with_capacity(n);
    for i in offset..offset + n {
        // Noms réalistes : préfixes répétés, nombres, extensions variées.
        let name = format!(
            "Rapport {} - v{} final ({}).{}",
            i % 977,
            i % 13,
            i,
            ["pdf", "docx", "jpg", "txt"][i % 4]
        );
        let wide: Vec<u16> = name.encode_utf16().collect();
        b.push_utf16(&wide, 0, (i * 7919 % 100_000) as u64, 0, i as i64);
    }
    b
}

fn bench(c: &mut Criterion) {
    c.bench_function("append 100k in one batch (sort naturel)", |b| {
        b.iter_batched(
            || batch(N, 0),
            |bt| {
                let mut s = DirSnapshot::new("C:\\bench".into(), 1, SortSpec::default());
                s.append(bt);
                black_box(s.len())
            },
            criterion::BatchSize::LargeInput,
        );
    });

    c.bench_function("append 100k in streaming batches", |b| {
        b.iter_batched(
            || {
                // Taille de lots croissante, comme fx-fs.
                let mut v = Vec::new();
                let (mut off, mut size) = (0, 256);
                while off < N {
                    let n = size.min(N - off);
                    v.push(batch(n, off));
                    off += n;
                    size = (size * 2).min(16_384);
                }
                v
            },
            |batches| {
                let mut s = DirSnapshot::new("C:\\bench".into(), 1, SortSpec::default());
                for bt in batches {
                    s.append(bt);
                }
                black_box(s.len())
            },
            criterion::BatchSize::LargeInput,
        );
    });

    let mut s = DirSnapshot::new("C:\\bench".into(), 1, SortSpec::default());
    s.append(batch(N, 0));
    c.bench_function("resort 100k by size desc then name asc", |b| {
        b.iter(|| {
            // Alterne d'après l'état courant : chaque itération fait un vrai re-tri.
            let spec = if s.sort().key == SortKey::Name {
                SortSpec {
                    key: SortKey::Size,
                    dir: SortDir::Desc,
                    folders_first: true,
                }
            } else {
                SortSpec::default()
            };
            s.set_sort(spec);
            black_box(s.len())
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
