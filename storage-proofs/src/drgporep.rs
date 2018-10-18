use std::marker::PhantomData;

use byteorder::{LittleEndian, WriteBytesExt};
use pairing::bls12_381::Fr;
use pairing::{PrimeField, PrimeFieldRepr};

use drgraph::Graph;
use error::Result;
use hasher::{Domain, Hasher};
use merkle::MerkleProof;
use parameter_cache::ParameterSetIdentifier;
use porep::{self, PoRep};
use proof::ProofScheme;
use util::data_at_node;
use vde::{self, decode_block};

#[derive(Debug)]
pub struct PublicInputs<T: Domain> {
    pub replica_id: T,
    pub challenges: Vec<usize>,
    pub tau: Option<porep::Tau<T>>,
}

#[derive(Debug)]
pub struct PrivateInputs<'a, H: 'a + Hasher> {
    pub replica: &'a [u8],
    pub aux: &'a porep::ProverAux<H>,
}

#[derive(Debug)]
pub struct SetupParams {
    pub lambda: usize,
    pub drg: DrgParams,
    pub sloth_iter: usize,
}

#[derive(Debug, Clone)]
pub struct DrgParams {
    // Number of nodes
    pub nodes: usize,

    // Base degree of DRG
    pub degree: usize,

    pub expansion_degree: usize,

    // Random seed
    pub seed: [u32; 7],
}

#[derive(Debug, Clone)]
pub struct PublicParams<H, G>
where
    H: Hasher,
    G: Graph<H> + ParameterSetIdentifier,
{
    pub lambda: usize,
    pub graph: G,
    pub sloth_iter: usize,

    _h: PhantomData<H>,
}

impl<H, G> PublicParams<H, G>
where
    H: Hasher,
    G: Graph<H> + ParameterSetIdentifier,
{
    pub fn new(lambda: usize, graph: G, sloth_iter: usize) -> Self {
        PublicParams {
            lambda,
            graph,
            sloth_iter,
            _h: PhantomData,
        }
    }
}

impl<H, G> ParameterSetIdentifier for PublicParams<H, G>
where
    H: Hasher,
    G: Graph<H> + ParameterSetIdentifier,
{
    fn parameter_set_identifier(&self) -> String {
        format!(
            "drgporep::PublicParams{{lambda: {}, graph: {}; sloth_iter: {}}}",
            self.lambda,
            self.graph.parameter_set_identifier(),
            self.sloth_iter
        )
    }
}

#[derive(Debug, Clone)]
pub struct DataProof<H: Hasher> {
    pub proof: MerkleProof<H>,
    pub data: H::Domain,
}

impl<H: Hasher> DataProof<H> {
    fn new(n: usize) -> Self {
        DataProof {
            proof: MerkleProof::new(n),
            data: Default::default(),
        }
    }
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = self.proof.serialize();
        let r: Fr = self.data.into();
        r.into_repr().write_le(&mut out).unwrap();

        out
    }

    /// proves_challenge returns true if this self.proof corresponds to challenge.
    /// This is useful for verifying that a supplied proof is actually relevant to a given challenge.
    pub fn proves_challenge(&self, challenge: usize) -> bool {
        let mut c = challenge;
        for (_, is_right) in self.proof.path().iter() {
            if ((c & 1) == 1) ^ is_right {
                return false;
            };
            c >>= 1;
        }
        true
    }
}

pub type ReplicaParents<H> = Vec<(usize, DataProof<H>)>;

#[derive(Default, Debug, Clone)]
pub struct Proof<H: Hasher> {
    pub replica_nodes: Vec<DataProof<H>>,
    pub replica_parents: Vec<ReplicaParents<H>>,
    pub nodes: Vec<DataProof<H>>,
}

impl<H: Hasher> Proof<H> {
    // FIXME: should we also take a number of challenges here and construct
    // vectors of that length?
    pub fn new_empty(height: usize, degree: usize) -> Proof<H> {
        Proof {
            replica_nodes: vec![DataProof::new(height)],
            replica_parents: vec![vec![(0, DataProof::new(height)); degree]],
            nodes: vec![DataProof::new(height)],
        }
    }
    pub fn serialize(&self) -> Vec<u8> {
        let res: Vec<_> = (0..self.nodes.len())
            .map(|i| {
                vec![
                    self.replica_nodes[i].serialize(),
                    self.replica_parents[i]
                        .iter()
                        .fold(Vec::new(), |mut acc, (s, p)| {
                            let mut v = vec![0u8; 4];
                            v.write_u32::<LittleEndian>(*s as u32).unwrap();
                            acc.extend(v);
                            acc.extend(p.serialize());
                            acc
                        }),
                    self.nodes[i].serialize(),
                ]
                .concat()
            })
            .collect::<Vec<Vec<u8>>>()
            .concat();

        res
    }

    pub fn new(
        replica_nodes: Vec<DataProof<H>>,
        replica_parents: Vec<ReplicaParents<H>>,
        nodes: Vec<DataProof<H>>,
    ) -> Proof<H> {
        Proof {
            replica_nodes,
            replica_parents,
            nodes,
        }
    }
}

impl<'a, H: Hasher> From<&'a Proof<H>> for Proof<H> {
    fn from(p: &Proof<H>) -> Proof<H> {
        Proof {
            replica_nodes: p.replica_nodes.clone(),
            replica_parents: p.replica_parents.clone(),
            nodes: p.nodes.clone(),
        }
    }
}

#[derive(Default)]
pub struct DrgPoRep<'a, H, G>
where
    H: 'a + Hasher,
    G: 'a + Graph<H>,
{
    _h: PhantomData<&'a H>,
    _g: PhantomData<G>,
}

impl<'a, H, G> ProofScheme<'a> for DrgPoRep<'a, H, G>
where
    H: 'a + Hasher,
    G: 'a + Graph<H> + ParameterSetIdentifier,
{
    type PublicParams = PublicParams<H, G>;
    type SetupParams = SetupParams;
    type PublicInputs = PublicInputs<H::Domain>;
    type PrivateInputs = PrivateInputs<'a, H>;
    type Proof = Proof<H>;

    fn setup(sp: &Self::SetupParams) -> Result<Self::PublicParams> {
        let graph = G::new(
            sp.drg.nodes,
            sp.drg.degree,
            sp.drg.expansion_degree,
            sp.drg.seed,
        );

        Ok(PublicParams::new(sp.lambda, graph, sp.sloth_iter))
    }

    fn prove<'b>(
        pub_params: &'b Self::PublicParams,
        pub_inputs: &'b Self::PublicInputs,
        priv_inputs: &'b Self::PrivateInputs,
    ) -> Result<Self::Proof> {
        let len = pub_inputs.challenges.len();

        let mut replica_nodes = Vec::with_capacity(len);
        let mut replica_parents = Vec::with_capacity(len);
        let mut data_nodes: Vec<DataProof<H>> = Vec::with_capacity(len);

        for i in 0..len {
            let challenge = pub_inputs.challenges[i] % pub_params.graph.size();
            assert_ne!(challenge, 0, "can not prove the first node");

            let tree_d = &priv_inputs.aux.tree_d;
            let tree_r = &priv_inputs.aux.tree_r;
            let replica = priv_inputs.replica;

            let data =
                H::Domain::try_from_bytes(data_at_node(replica, challenge, pub_params.lambda)?)?;

            replica_nodes.push(DataProof {
                proof: MerkleProof::new_from_proof(&tree_r.gen_proof(challenge)),
                data,
            });

            let parents = pub_params.graph.parents(challenge);
            let mut replica_parentsi = Vec::with_capacity(parents.len());

            for p in parents {
                replica_parentsi.push((p, {
                    let proof = tree_r.gen_proof(p);
                    DataProof {
                        proof: MerkleProof::new_from_proof(&proof),
                        data: H::Domain::try_from_bytes(data_at_node(
                            replica,
                            p,
                            pub_params.lambda,
                        )?)?,
                    }
                }));
            }

            replica_parents.push(replica_parentsi);

            let node_proof = tree_d.gen_proof(challenge);

            {
                // TODO: use this again, I can't make lifetimes work though atm and I do not know why
                // let extracted = Self::extract(
                //     pub_params,
                //     &pub_inputs.replica_id.into_bytes(),
                //     &replica,
                //     challenge,
                // )?;

                let extracted = decode_block(
                    &pub_params.graph,
                    pub_params.lambda,
                    pub_params.sloth_iter,
                    &pub_inputs.replica_id,
                    &replica,
                    challenge,
                )?
                .into_bytes();
                data_nodes.push(DataProof {
                    data: H::Domain::try_from_bytes(&extracted)?,
                    proof: MerkleProof::new_from_proof(&node_proof),
                });
            }
        }

        let proof = Proof::new(replica_nodes, replica_parents, data_nodes);

        Ok(proof)
    }

    fn verify(
        pub_params: &Self::PublicParams,
        pub_inputs: &Self::PublicInputs,
        proof: &Self::Proof,
    ) -> Result<bool> {
        for i in 0..pub_inputs.challenges.len() {
            {
                // This was verify_proof_meta.
                if pub_inputs.challenges[i] >= pub_params.graph.size() {
                    return Ok(false);
                }

                if !(proof.nodes[i].proves_challenge(pub_inputs.challenges[i])) {
                    return Ok(false);
                }

                if !(proof.replica_nodes[i].proves_challenge(pub_inputs.challenges[i])) {
                    return Ok(false);
                }

                let expected_parents = pub_params.graph.parents(pub_inputs.challenges[i]);
                if proof.replica_parents[i].len() != expected_parents.len() {
                    println!(
                        "proof parents were not the same length as in public parameters: {} != {}",
                        proof.replica_parents[i].len(),
                        expected_parents.len()
                    );
                    return Ok(false);
                }

                let parents_as_expected = proof.replica_parents[i]
                    .iter()
                    .zip(&expected_parents)
                    .all(|(actual, expected)| actual.0 == *expected);

                if !parents_as_expected {
                    println!("proof parents were not those provided in public parameters");
                    return Ok(false);
                }
            }

            let challenge = pub_inputs.challenges[i] % pub_params.graph.size();
            assert_ne!(challenge, 0, "can not prove the first node");

            if !proof.replica_nodes[i].proof.validate(challenge) {
                println!("invalid replica node");
                return Ok(false);
            }

            for (parent_node, p) in &proof.replica_parents[i] {
                if !p.proof.validate(*parent_node) {
                    println!("invalid replica parent: {:?}", p);
                    return Ok(false);
                }
            }

            let prover_bytes = &pub_inputs.replica_id.into_bytes();

            let key_input =
                proof.replica_parents[i]
                    .iter()
                    .fold(prover_bytes.clone(), |mut acc, (_, p)| {
                        acc.extend(&p.data.into_bytes());
                        acc
                    });

            let key = H::kdf(key_input.as_slice(), pub_params.graph.degree());
            let unsealed =
                H::sloth_decode(&key, &proof.replica_nodes[i].data, pub_params.sloth_iter);

            if unsealed != proof.nodes[i].data {
                return Ok(false);
            }

            if !proof.nodes[i].proof.validate_data(&unsealed) {
                println!("invalid data for merkle path{:?}", unsealed);
                return Ok(false);
            }
        }

        Ok(true)
    }
}

impl<'a, H, G> PoRep<'a, H::Domain> for DrgPoRep<'a, H, G>
where
    H: 'a + Hasher,
    G: 'a + Graph<H> + ParameterSetIdentifier,
{
    type Tau = porep::Tau<H::Domain>;
    type ProverAux = porep::ProverAux<H>;

    fn replicate(
        pp: &Self::PublicParams,
        replica_id: &H::Domain,
        data: &mut [u8],
    ) -> Result<(porep::Tau<H::Domain>, porep::ProverAux<H>)> {
        let tree_d = pp.graph.merkle_tree(data, pp.lambda)?;
        let comm_d = tree_d.root();

        vde::encode(&pp.graph, pp.lambda, pp.sloth_iter, replica_id, data)?;

        let tree_r = pp.graph.merkle_tree(data, pp.lambda)?;
        let comm_r = tree_r.root();

        Ok((
            porep::Tau::new(comm_d, comm_r),
            porep::ProverAux::new(tree_d, tree_r),
        ))
    }

    fn extract_all<'b>(
        pp: &'b Self::PublicParams,
        replica_id: &'b H::Domain,
        data: &'b [u8],
    ) -> Result<Vec<u8>> {
        vde::decode(&pp.graph, pp.lambda, pp.sloth_iter, replica_id, data)
    }

    fn extract(
        pp: &Self::PublicParams,
        replica_id: &H::Domain,
        data: &[u8],
        node: usize,
    ) -> Result<Vec<u8>> {
        Ok(decode_block(&pp.graph, pp.lambda, pp.sloth_iter, replica_id, data, node)?.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use memmap::MmapMut;
    use memmap::MmapOptions;
    use pairing::bls12_381::Bls12;
    use rand::{Rng, SeedableRng, XorShiftRng};
    use std::fs::File;
    use std::io::Write;
    use tempfile;

    use drgraph::{new_seed, BucketGraph};
    use fr32::fr_into_bytes;
    use hasher::pedersen::*;

    pub fn file_backed_mmap_from(data: &[u8]) -> MmapMut {
        let mut tmpfile: File = tempfile::tempfile().unwrap();
        tmpfile.write_all(data).unwrap();

        unsafe { MmapOptions::new().map_mut(&tmpfile).unwrap() }
    }

    #[test]
    fn extract_all() {
        let rng = &mut XorShiftRng::from_seed([0x3dbe6259, 0x8d313d76, 0x3237db17, 0xe5bc0654]);

        let lambda = 32;
        let sloth_iter = 1;
        let replica_id: Fr = rng.gen();
        let data = vec![2u8; 32 * 3];
        // create a copy, so we can compare roundtrips
        let mut mmapped_data_copy = file_backed_mmap_from(&data);

        let sp = SetupParams {
            lambda,
            drg: DrgParams {
                nodes: data.len() / lambda,
                degree: 5,
                expansion_degree: 0,
                seed: new_seed(),
            },
            sloth_iter,
        };

        let pp = DrgPoRep::<PedersenHasher, BucketGraph<_>>::setup(&sp).unwrap();

        DrgPoRep::replicate(&pp, &replica_id.into(), &mut mmapped_data_copy).unwrap();

        let mut copied = vec![0; data.len()];
        copied.copy_from_slice(&mmapped_data_copy);
        assert_ne!(data, copied, "replication did not change data");

        let decoded_data =
            DrgPoRep::extract_all(&pp, &replica_id.into(), &mut mmapped_data_copy).unwrap();

        assert_eq!(data, decoded_data.as_slice(), "failed to extract data");
    }

    #[test]
    fn extract() {
        let rng = &mut XorShiftRng::from_seed([0x3dbe6259, 0x8d313d76, 0x3237db17, 0xe5bc0654]);

        let lambda = 32;
        let sloth_iter = 1;
        let replica_id: Fr = rng.gen();
        let nodes = 3;
        let data = vec![2u8; 32 * nodes];

        // create a copy, so we can compare roundtrips
        let mut mmapped_data_copy = file_backed_mmap_from(&data);

        let sp = SetupParams {
            lambda,
            drg: DrgParams {
                nodes: data.len() / lambda,
                degree: 5,
                expansion_degree: 0,
                seed: new_seed(),
            },
            sloth_iter,
        };

        let pp = DrgPoRep::<PedersenHasher, BucketGraph<_>>::setup(&sp).unwrap();

        DrgPoRep::replicate(&pp, &replica_id.into(), &mut mmapped_data_copy).unwrap();

        let mut copied = vec![0; data.len()];
        copied.copy_from_slice(&mmapped_data_copy);
        assert_ne!(data, copied, "replication did not change data");

        for i in 0..nodes {
            let decoded_data =
                DrgPoRep::extract(&pp, &replica_id.into(), &mmapped_data_copy, i).unwrap();

            let original_data = data_at_node(&data, i, lambda).unwrap();

            assert_eq!(
                original_data,
                decoded_data.as_slice(),
                "failed to extract data"
            );
        }
    }

    fn prove_verify_aux(
        lambda: usize,
        nodes: usize,
        i: usize,
        use_wrong_challenge: bool,
        use_wrong_parents: bool,
    ) {
        assert!(i < nodes);

        let mut repeat = true;
        while repeat {
            repeat = false;

            let rng = &mut XorShiftRng::from_seed([0x3dbe6259, 0x8d313d76, 0x3237db17, 0xe5bc0654]);
            let sloth_iter = 1;
            let degree = 10;
            let expansion_degree = 0;
            let seed = new_seed();

            let replica_id: Fr = rng.gen();
            let data: Vec<u8> = (0..nodes)
                .flat_map(|_| fr_into_bytes::<Bls12>(&rng.gen()))
                .collect();

            // create a copy, so we can comare roundtrips
            let mut mmapped_data_copy = file_backed_mmap_from(&data);

            let challenge = i;

            let sp = SetupParams {
                lambda,
                drg: DrgParams {
                    nodes,
                    degree,
                    expansion_degree,
                    seed,
                },
                sloth_iter,
            };

            let pp = DrgPoRep::<PedersenHasher, BucketGraph<_>>::setup(&sp).unwrap();

            let (tau, aux) = DrgPoRep::<PedersenHasher, _>::replicate(
                &pp,
                &replica_id.into(),
                &mut mmapped_data_copy,
            )
            .unwrap();

            let mut copied = vec![0; data.len()];
            copied.copy_from_slice(&mmapped_data_copy);

            assert_ne!(data, copied, "replication did not change data");

            let pub_inputs = PublicInputs::<PedersenDomain> {
                replica_id: replica_id.into(),
                challenges: vec![challenge, challenge],
                tau: Some(tau.clone().into()),
            };

            let priv_inputs = PrivateInputs::<PedersenHasher> {
                replica: &mmapped_data_copy,
                aux: &aux,
            };

            let real_proof =
                DrgPoRep::<PedersenHasher, _>::prove(&pp, &pub_inputs, &priv_inputs).unwrap();

            if use_wrong_parents {
                // Only one 'wrong' option will be tested at a time.
                assert!(!use_wrong_challenge);
                let real_parents = real_proof.replica_parents;

                // Parent vector claiming the wrong parents.
                let fake_parents = vec![
                    real_parents[0]
                        .iter()
                        // Incrementing each parent node will give us a different parent set.
                        // It's fine to be out of range, since this only needs to fail.
                        .map(|(i, data_proof)| (i + 1, data_proof.clone()))
                        .collect::<Vec<_>>(),
                ];

                let proof = Proof::new(
                    real_proof.replica_nodes.clone(),
                    fake_parents,
                    real_proof.nodes.clone().into(),
                );

                assert!(
                    !DrgPoRep::verify(&pp, &pub_inputs, &proof).unwrap(),
                    "verified in error -- with wrong parents"
                );

                let mut all_same = true;
                for (p, _) in &real_parents[0] {
                    if *p != real_parents[0][0].0 {
                        all_same = false;
                    }
                }

                if all_same {
                    println!("invalid test data can't scramble proofs with all same parents.");

                    repeat = true;
                    continue;
                }

                // Parent vector claiming the right parents but providing valid proofs for different
                // parents.
                let fake_proof_parents = vec![
                    real_parents[0]
                        .iter()
                        .enumerate()
                        .map(|(i, (p, _))| {
                            // Rotate the real parent proofs.
                            let x = (i + 1) % real_parents[0].len();
                            let j = real_parents[0][x].0;
                            (*p, real_parents[0][j].1.clone())
                        })
                        .collect::<Vec<_>>(),
                ];

                let proof2 = Proof::new(
                    real_proof.replica_nodes,
                    fake_proof_parents,
                    real_proof.nodes.into(),
                );

                assert!(
                    !DrgPoRep::<PedersenHasher, _>::verify(&pp, &pub_inputs, &proof2).unwrap(),
                    "verified in error -- with wrong parent proofs"
                );

                return ();
            }

            let proof = real_proof;

            if use_wrong_challenge {
                let pub_inputs_with_wrong_challenge_for_proof = PublicInputs::<PedersenDomain> {
                    replica_id: replica_id.into(),
                    challenges: vec![if challenge == 1 { 2 } else { 1 }],
                    tau: Some(tau.into()),
                };
                let verified = DrgPoRep::<PedersenHasher, _>::verify(
                    &pp,
                    &pub_inputs_with_wrong_challenge_for_proof,
                    &proof,
                )
                .unwrap();
                assert!(
                    !verified,
                    "wrongly verified proof which does not match challenge in public input"
                );
            } else {
                assert!(
                    DrgPoRep::<PedersenHasher, _>::verify(&pp, &pub_inputs, &proof).unwrap(),
                    "failed to verify"
                );
            }
        }
    }

    fn prove_verify(lambda: usize, n: usize, i: usize) {
        prove_verify_aux(lambda, n, i, false, false)
    }

    fn prove_verify_wrong_challenge(lambda: usize, n: usize, i: usize) {
        prove_verify_aux(lambda, n, i, true, false)
    }

    fn prove_verify_wrong_parents(lambda: usize, n: usize, i: usize) {
        prove_verify_aux(lambda, n, i, false, true)
    }

    table_tests!{
        prove_verify {
            prove_verify_32_2_1(32, 2, 1);

            prove_verify_32_3_1(32, 3, 1);
            prove_verify_32_3_2(32, 3, 2);

            prove_verify_32_10_1(32, 10, 1);
            prove_verify_32_10_2(32, 10, 2);
            prove_verify_32_10_3(32, 10, 3);
            prove_verify_32_10_4(32, 10, 4);
            prove_verify_32_10_5(32, 10, 5);
        }
    }

    #[test]
    fn test_drgporep_verifies_using_challenge() {
        prove_verify_wrong_challenge(32, 5, 1);
    }

    #[test]
    fn test_drgporep_verifies_parents() {
        // Challenge a node (3) that doesn't have all the same parents.
        prove_verify_wrong_parents(32, 7, 4);
    }

}
