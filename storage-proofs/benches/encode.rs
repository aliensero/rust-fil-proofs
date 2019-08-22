#[macro_use]
extern crate criterion;

use algebra::curves::bls12_377::Bls12_377 as Bls12;
use criterion::{black_box, Criterion, ParameterizedBenchmark};
use rand::{thread_rng, Rng};
use storage_proofs::fr32::fr_into_bytes;
use storage_proofs::hasher::{Blake2sHasher, PedersenHasher};
use storage_proofs::hasher::{Domain, Hasher};
use storage_proofs::util::{data_at_node_offset, NODE_SIZE};
use storage_proofs::vde;

struct Pregenerated<H: Hasher> {
    data: Vec<u8>,
    parents: Vec<usize>,
    replica_id: H::Domain,
}

fn pregenerate_data<H: Hasher>(degree: usize) -> Pregenerated<H> {
    let mut rng = thread_rng();
    let data: Vec<u8> = (0..(degree + 1))
        .flat_map(|_| fr_into_bytes::<Bls12>(&rng.gen()))
        .collect();
    let parents: Vec<usize> = (0..degree).map(|pos| pos).collect();
    let replica_id: H::Domain = rng.gen();
    Pregenerated {
        data,
        parents,
        replica_id,
    }
}

fn encode_single_node<H: Hasher>(
    data: &mut [u8],
    parents: &[usize],
    replica_id: &H::Domain,
    node: usize,
) {
    let key = vde::create_key::<H>(replica_id, node, parents, data).unwrap();
    let start = data_at_node_offset(node);
    let end = start + NODE_SIZE;

    let node_data = H::Domain::try_from_bytes(&data[start..end]).unwrap();
    let encoded = H::sloth_encode(&key, &node_data);
    encoded.write_bytes(&mut data[start..end]).unwrap();
}

fn kdf_benchmark(c: &mut Criterion) {
    let degrees = vec![3, 5, 10];

    c.bench(
        "kdf",
        ParameterizedBenchmark::new(
            "blake2s",
            |b, degree| {
                let Pregenerated {
                    mut data,
                    parents,
                    replica_id,
                } = pregenerate_data::<Blake2sHasher>(*degree);
                b.iter(|| {
                    black_box(vde::create_key::<Blake2sHasher>(
                        &replica_id,
                        *degree,
                        &parents,
                        &mut data,
                    ))
                })
            },
            degrees,
        )
        .with_function("pedersen", |b, degree| {
            let Pregenerated {
                mut data,
                parents,
                replica_id,
            } = pregenerate_data::<PedersenHasher>(*degree);
            b.iter(|| {
                black_box(vde::create_key::<PedersenHasher>(
                    &replica_id,
                    *degree,
                    &parents,
                    &mut data,
                ))
            })
        }), // .with_function("sha256", |b, degree| {
            //     let Pregenerated {
            //         mut data,
            //         parents,
            //         replica_id,
            //     } = pregenerate_data::<Sha256Hasher>(*degree);
            //     b.iter(|| {
            //         black_box(encode_single_node::<Sha256Hasher>(
            //             &mut data,
            //             &parents,
            //             &replica_id,
            //             *degree,
            //         ))
            //     })
            // }),
    );
}

fn encode_single_node_benchmark(c: &mut Criterion) {
    let degrees = vec![3, 5, 10];

    c.bench(
        "encode-node",
        ParameterizedBenchmark::new(
            "blake2s",
            |b, degree| {
                let Pregenerated {
                    mut data,
                    parents,
                    replica_id,
                } = pregenerate_data::<Blake2sHasher>(*degree);
                b.iter(|| {
                    black_box(encode_single_node::<Blake2sHasher>(
                        &mut data,
                        &parents,
                        &replica_id,
                        *degree,
                    ))
                })
            },
            degrees,
        )
        .with_function("pedersen", |b, degree| {
            let Pregenerated {
                mut data,
                parents,
                replica_id,
            } = pregenerate_data::<PedersenHasher>(*degree);
            b.iter(|| {
                black_box(encode_single_node::<PedersenHasher>(
                    &mut data,
                    &parents,
                    &replica_id,
                    *degree,
                ))
            })
        }), // .with_function("sha256", |b, degree| {
            //     let Pregenerated {
            //         mut data,
            //         parents,
            //         replica_id,
            //     } = pregenerate_data::<Sha256Hasher>(*degree);
            //     b.iter(|| {
            //         black_box(encode_single_node::<Sha256Hasher>(
            //             &mut data,
            //             &parents,
            //             &replica_id,
            //             *degree,
            //         ))
            //     })
            // }),
    );
}

criterion_group!(benches, encode_single_node_benchmark, kdf_benchmark);
criterion_main!(benches);
