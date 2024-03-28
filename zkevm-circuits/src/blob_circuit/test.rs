use eth_types::{
    sign_types::{sign, SignData},
    Field, U256,
};
use halo2_base::utils::fe_to_biguint;
use halo2_proofs::{
    arithmetic::Field as HaloField, circuit, dev::MockProver, halo2curves::{
        bn256::Fr,
        group::Curve,
        secp256k1::{self, Secp256k1Affine}, FieldExt,
    }
};
use rand::{Rng, RngCore};
use std::marker::PhantomData;
use bls12_381::{Scalar as Fp};
use crate::{blob_circuit::BlobCircuit, util::SubCircuit};
use rand::rngs::OsRng;

use crate::blob_circuit::util::*;

#[test]
fn test_blob_consistency(){
    let batch_commit = Fr::random(OsRng);

    let challenge_point = Fp::random(OsRng);
    let blob: Vec<Fp> = (0..4096)
        .map(|_| Fp::random(OsRng))
        .collect();

    println!("blob:{:?}",blob);

    let omega = blob_width_th_root_of_unity();
    println!("omega:{}", omega);
    let result = poly_eval(blob.clone(), challenge_point, omega);

    println!("real result:{}", result);

    println!("U256:{:?}", U256::from_little_endian(&result.to_bytes()));



    let circuit_blob = CircuitBlob::<Fr>::new(challenge_point, 0, blob.clone(), result);

    let circuit = BlobCircuit::<Fr> {
        batch_commit: batch_commit,
        challenge_point: challenge_point,
        index: 0,
        partial_blob: blob.clone(),
        partial_result: result,
        exports: std::cell::RefCell::new(None),
        _marker: PhantomData,
    };    

    let instance = circuit.instance();

    let prover = match MockProver::<Fr>::run(19, &circuit, instance) {
        Ok(prover) => prover,
        Err(e) => panic!("{e:#?}"),
    };

    assert_eq!(prover.verify(), Ok(()));
}

#[test]
fn test_partial_blob_consistency(){
    let blob: Vec<Fp> = (0..51)
        .map(|_| Fp::random(OsRng))
        .collect();
    

    log::trace!("blob:{:?}", blob);

    let index = 50;
    let omega = blob_width_th_root_of_unity();

    // let challenge_point = roots_of_unity_brp[0];
    let challenge_point = Fp::random(OsRng);
    // let challenge_point = Fp::from(128);

    let result = poly_eval_partial(blob.clone(), challenge_point, omega, index);
    
    log::trace!("real result:{}", result);

    let circuit_blob = CircuitBlob::<Fr>::new(challenge_point, index, blob.clone(), result);

    let circuit = BlobCircuit::<Fr> {
        blob:circuit_blob,
        exports: std::cell::RefCell::new(None),
        _marker: PhantomData::default(),
    };    

    let instance = circuit.instance();

    let prover = match MockProver::<Fr>::run(20, &circuit, instance) {
        Ok(prover) => prover,
        Err(e) => panic!("{e:#?}"),
    };

    assert_eq!(prover.verify(), Ok(()));
}

#[test]
fn test_zero_blob(){
    let blob: Vec<u8> = vec![0; 32 * 4096];
    let mut result: Vec<Fp> = Vec::new();
    for chunk in blob.chunks(32) {
        let reverse: Vec<u8> = chunk.iter().rev().cloned().collect();  
        result.push(Fp::from_bytes(reverse.as_slice().try_into().unwrap()).unwrap());
    }

    
    log::trace!("partial blob: {:?}  len: {:?}", result, result.len());
}

#[test]
fn test_root_of_unity(){

    let modulus = U256::from_str_radix(Fp::MODULUS, 16).unwrap();

    let exponent = (modulus - U256::one()) / U256::from(4096);

    let primitive_root_of_unity = Fp::from(7);

    let root_of_unity = primitive_root_of_unity.pow(&exponent.0);

    println!("root of unity= {:?}", root_of_unity);
}