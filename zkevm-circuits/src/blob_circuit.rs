use halo2_base::{
    Context,
    utils::{
        ScalarField, fe_to_biguint, modulus, decompose_biguint,}, 
    gates::GateInstructions, AssignedValue,
};

use halo2_ecc::fields::{fp::{FpConfig, FpStrategy}, FieldChip};
use halo2_proofs::{
    circuit::{Layouter, Value},
    plonk::{ConstraintSystem, Error, Expression, Column, Instance},
};

use bls12_381::Scalar as Fp;
use itertools::Itertools;
use crate::{util::{SubCircuit, Challenges, SubCircuitConfig}, witness::Block};
use std::{io::Read, marker::PhantomData};
use eth_types::{Field, ToBigEndian, ToLittleEndian, ToScalar, H256};
use rand::rngs::OsRng;

mod util;
mod test;
mod dev;

use util::*;

// BLOB_WIDTH must be a power of two
pub const BLOB_WIDTH: usize = 4096;
pub const BLOB_WIDTH_BITS: u32 = 12;

pub const K: usize = 14;
pub const LOOKUP_BITS: usize = 10;


#[derive(Clone, Debug)]
pub struct BlobCircuitConfigArgs<F: Field> {
    /// zkEVM challenge API.
    pub challenges: Challenges<Expression<F>>,
}

/// blob circuit config
#[derive(Clone, Debug)]
pub struct BlobCircuitConfig<F: Field> {
    /// Field config for bls12-381::Scalar.
    fp_config: FpConfig<F, Fp>,
    instance: Column<Instance>,
    /// Number of limbs to represent Fp.
    num_limbs: usize,
    /// Number of bits per limb.
    limb_bits: usize,
    _marker: PhantomData<F>,
}

/// BlobCircuit
#[derive(Default, Clone, Debug)]
pub struct BlobCircuit<F: Field> {
    /// commit of batch
    pub batch_commit: F,
    /// challenge point x
    pub challenge_point: Fp,
    /// index of blob element    
    pub index: usize,
    /// partial blob element    
    pub partial_blob: Vec<Fp>,
    /// partial result
    pub partial_result: Fp,
    _marker: PhantomData<F>,
}

impl<F: Field> BlobCircuit<F> {
    /// Return a new BlobCircuit
    pub fn new(batch_commit:F, challenge_point:Fp, index:usize, partial_blob:Vec<Fp>, partial_result: Fp) -> Self {
        Self {
            batch_commit,
            challenge_point,
            index,
            partial_blob,
            partial_result,
            _marker: PhantomData::default(),
        }
    }

    pub fn partial_blob(block: &Block<F>) -> Vec<Fp> {
        match block_to_blob(block) {
            Ok(blob) => {
                let mut result: Vec<Fp> = Vec::new();
                for chunk in blob.chunks(32) {
                    let reverse: Vec<u8> = chunk.iter().rev().cloned().collect();  
                    result.push(Fp::from_bytes(reverse.as_slice().try_into().unwrap()).unwrap());
                }
                result
            }
            Err(_) => Vec::new(),
        }
    }
}


impl<F: Field> SubCircuitConfig<F> for BlobCircuitConfig<F>{
    type ConfigArgs = BlobCircuitConfigArgs<F>;
    fn new(
        meta: &mut ConstraintSystem<F>,
        Self::ConfigArgs {
            challenges: _,
        }: Self::ConfigArgs,
    ) -> Self {
        let num_limbs = 3;
        let limb_bits = 88;
        #[cfg(feature = "onephase")]
        let num_advice = [35];
        #[cfg(not(feature = "onephase"))]
        let num_advice = [35, 1];

        let fp_config = FpConfig::configure(
            meta,
            FpStrategy::Simple,
            &num_advice,
            &[17], // num lookup advice
            1,     // num fixed
            10,    // lookup bits
            limb_bits,
            num_limbs,
            modulus::<Fp>(),
            0,
            19, // k
        );

        let instance = meta.instance_column();
        meta.enable_equality(instance);
        
        Self {
            fp_config,
            instance,
            num_limbs,
            limb_bits,
            _marker: PhantomData,
        }
    }
} 

impl<F: Field> BlobCircuit<F>{
    pub(crate) fn assign(
        &self,
        ctx: &mut Context<F>,
        fp_chip: &FpConfig<F, Fp>,
        challenges: &Challenges<Value<F>>,
    ) ->  Result<Vec<AssignedValue<F>>, Error>{

        let gate = &fp_chip.range.gate;

        let one_fp = fp_chip.load_constant(ctx, fe_to_biguint(&Fp::one()));

        let zero = gate.load_zero(ctx);

        //load challenge_point to fp_chip
        let challenge_point_fp = load_private(fp_chip, ctx, Value::known(self.challenge_point));

        let blob = self
            .partial_blob
            .iter()
            .map(|x| load_private(fp_chip, ctx, Value::known(*x)))
            .collect::<Vec<_>>();

        let partial_blob_len = blob.len();
        log::trace!("partial blob len{}", partial_blob_len);
        // === STEP 2: compute the barycentric formula ===
        // spec reference:
        // https://github.com/ethereum/consensus-specs/blob/dev/specs/deneb/polynomial-commitments.md
        //
        // barycentric formula:
        // Evaluate a polynomial (in evaluation form) at an arbitrary point ``z``.
        // - When ``z`` is in the domain, the evaluation can be found by indexing
        // the polynomial at the position that ``z`` is in the domain.
        // - When ``z`` is not in the domain, the barycentric formula is used:
        //    f(z) = ((z**WIDTH - 1) / WIDTH) *  sum_(i=0)^WIDTH  (f(DOMAIN[i]) * DOMAIN[i]) / (z - DOMAIN[i])
        //
        // In our case:
        // - ``z`` is the challenge point in Fp
        // - ``WIDTH`` is BLOB_WIDTH
        // - ``DOMAIN`` is the bit_reversal_permutation roots of unity
        // - ``f(DOMAIN[i])`` is the blob[i]

        
        // let (cp_lo, cp_hi) = decompose_to_lo_hi(ctx, &fp_chip.range, challenge_point);
            
        // let challenge_point_fp = cross_field_load_private(ctx, &fp_chip, &fp_chip.range, &cp_lo, &cp_hi);

        // loading roots of unity to fp_chip as constants
        
        let blob_width_th_root_of_unity =
        Fp::from(123).pow(&[(FP_S - BLOB_WIDTH_BITS) as u64, 0, 0, 0]);
        // let blob_width_th_root_of_unity = get_omega(4, 2);
        let roots_of_unity: Vec<_> = (0..BLOB_WIDTH)
            .map(|i| blob_width_th_root_of_unity.pow(&[i as u64, 0, 0, 0]))
            .collect();
        let roots_of_unity = roots_of_unity
            .iter()
            .map(|x| fp_chip.load_constant(ctx, fe_to_biguint(x)))
            .collect::<Vec<_>>();          

        // let roots_of_unity_brp = roots_of_unity;
        // apply bit_reversal_permutation to roots_of_unity
        // spec reference:
        // https://github.com/ethereum/consensus-specs/blob/dev/specs/deneb/polynomial-commitments.md#bit-reversal-permutation
        //
        let roots_of_unity_brp = bit_reversal_permutation(roots_of_unity);

        let mut result = fp_chip.load_constant(ctx, fe_to_biguint(&Fp::zero()));
        let mut cp_is_not_root_of_unity = fp_chip.load_constant(ctx, fe_to_biguint(&Fp::one()));
        let mut barycentric_evaluation = fp_chip.load_constant(ctx, fe_to_biguint(&Fp::zero()));
        

        for i in 0..partial_blob_len as usize {
            let numinator_i = fp_chip.mul(ctx, &roots_of_unity_brp[i + self.index].clone(), &blob[i].clone());
    
            let denominator_i_no_carry = fp_chip.sub_no_carry(
                ctx,
                &challenge_point_fp.clone(),
                &roots_of_unity_brp[i + self.index].clone(),
            );
            let denominator_i = fp_chip.carry_mod(ctx, &denominator_i_no_carry);
            // avoid division by zero
            // safe_denominator_i = denominator_i       (denominator_i != 0)
            // safe_denominator_i = 1                   (denominator_i == 0)
            let is_zero_denominator_i = fp_is_zero(ctx, &gate, &denominator_i);
            let is_zero_denominator_i =
                cross_field_load_private(ctx, &fp_chip, &fp_chip.range, &is_zero_denominator_i, &zero);
            // let is_zero_denominator_i = fp_chip.load_private(ctx, Value::known(fe_to_bigint(&Fp::zero())));
            let safe_denominator_i =
                fp_chip.add_no_carry(ctx, &denominator_i, &is_zero_denominator_i.clone());
            let safe_denominator_i = fp_chip.carry_mod(ctx, &safe_denominator_i);

            // update `cp_is_not_root_of_unity`
            // cp_is_not_root_of_unity = 1          (initialize)
            // cp_is_not_root_of_unity = 0          (denominator_i == 0)
            let non_zero_denominator_i =
                fp_chip.sub_no_carry(ctx, &one_fp.clone(), &is_zero_denominator_i.clone());
            cp_is_not_root_of_unity = fp_chip.mul(ctx, &cp_is_not_root_of_unity, &non_zero_denominator_i);

            // update `result`
            // result = blob[i]     (challenge_point = roots_of_unity_brp[i])
            let select_blob_i = fp_chip.mul(ctx, &blob[i].clone(), &is_zero_denominator_i.clone());
            let tmp_result = fp_chip.add_no_carry(ctx, &result, &select_blob_i);
            result = fp_chip.carry_mod(ctx, &tmp_result);

            let term_i = fp_chip.divide(ctx, &numinator_i, &safe_denominator_i);
            let evaluation_not_proper = fp_chip.add_no_carry(ctx, &barycentric_evaluation, &term_i);
            barycentric_evaluation = fp_chip.carry_mod(ctx, &evaluation_not_proper);
        }
        let cp_to_the_width = fp_pow(ctx, &fp_chip, &challenge_point_fp, BLOB_WIDTH as u32);
        let cp_to_the_width_minus_one = fp_chip.sub_no_carry(ctx, &cp_to_the_width, &one_fp);
        let cp_to_the_width_minus_one = fp_chip.carry_mod(ctx, &cp_to_the_width_minus_one);
        let width_fp = fp_chip.load_constant(ctx, fe_to_biguint(&Fp::from(BLOB_WIDTH as u64)));
        let factor = fp_chip.divide(ctx, &cp_to_the_width_minus_one, &width_fp);
        barycentric_evaluation = fp_chip.mul(ctx, &barycentric_evaluation, &factor);
    
        // === STEP 3: select between the two case ===
        // if challenge_point is a root of unity(index..index + partial_blob_len), then result = blob[i]
        // if challenge_point is a root of unity((0..self.index)or((self.index + partial_blob_len)..BLOB_WIDTH), then result = 0
        // else result = barycentric_evaluation
        for i in (0..self.index).chain((self.index + partial_blob_len)..BLOB_WIDTH) {
            let denominator_i_no_carry = fp_chip.sub_no_carry(
                ctx,
                &challenge_point_fp.clone(),
                &roots_of_unity_brp[i].clone(),
            );
            let denominator_i = fp_chip.carry_mod(ctx, &denominator_i_no_carry);
            // avoid division by zero
            // safe_denominator_i = denominator_i       (denominator_i != 0)
            // safe_denominator_i = 1                   (denominator_i == 0)
            let is_zero_denominator_i = fp_is_zero(ctx, &gate, &denominator_i);
            let is_zero_denominator_i =
                cross_field_load_private(ctx, &fp_chip, &fp_chip.range, &is_zero_denominator_i, &zero);
            // update `cp_is_not_root_of_unity`
            // cp_is_not_root_of_unity = 1          (initialize)
            // cp_is_not_root_of_unity = 0          (denominator_i == 0)
            let non_zero_denominator_i =
                fp_chip.sub_no_carry(ctx, &one_fp.clone(), &is_zero_denominator_i.clone());
            cp_is_not_root_of_unity = fp_chip.mul(ctx, &cp_is_not_root_of_unity, &non_zero_denominator_i);
        }
        
        let select_evaluation = fp_chip.mul(ctx, &barycentric_evaluation, &cp_is_not_root_of_unity);
        let tmp_result = fp_chip.add_no_carry(ctx, &result, &select_evaluation);
        result = fp_chip.carry_mod(ctx, &tmp_result);

        log::trace!("limb 1 \n barycentric_evaluation {:?}", barycentric_evaluation.truncation.limbs[0].value());
        log::trace!("limb 2 \n barycentric_evaluation {:?}", barycentric_evaluation.truncation.limbs[1].value());
        log::trace!("limb 3 \n barycentric_evaluation {:?}", barycentric_evaluation.truncation.limbs[2].value());

        log::trace!("limb 1 \n reconstructed {:?}", result.truncation.limbs[0].value());
        log::trace!("limb 2 \n reconstructed {:?}", result.truncation.limbs[1].value());
        log::trace!("limb 3 \n reconstructed {:?}", result.truncation.limbs[2].value());

        let result = vec![challenge_point_fp.truncation.limbs[0], challenge_point_fp.truncation.limbs[1], challenge_point_fp.truncation.limbs[2], result.truncation.limbs[0], result.truncation.limbs[1], result.truncation.limbs[2]];
        
        Ok(result)
    }
}


impl<F: Field> SubCircuit<F> for BlobCircuit<F>{
    type Config = BlobCircuitConfig<F>;


    fn new_from_block(block: &Block<F>) -> Self {
        Self{
            batch_commit: block.batch_commit.to_scalar().unwrap(), 
            challenge_point: Fp::from_bytes(&block.challenge_point.to_le_bytes()).unwrap(),
            index: block.index,
            partial_blob: Self::partial_blob(block),
            partial_result: Fp::from_bytes(&block.partial_result.to_le_bytes()).unwrap(),
            _marker: Default::default(),
        }
    }

    fn min_num_rows_block(block: &Block<F>) -> (usize, usize) {
        (1<<19,1<<19)
    }

    /// Compute the public inputs for this circuit.
    fn instance(&self) -> Vec<Vec<F>> {

        let omega = Fp::from(123).pow(&[(FP_S - 12) as u64, 0, 0, 0]);

        let result = poly_eval_partial(self.partial_blob.clone(), self.challenge_point, omega, self.index);

        let mut public_inputs = decompose_biguint(&fe_to_biguint(&self.challenge_point), NUM_LIMBS, LIMB_BITS);

        public_inputs.extend(decompose_biguint::<F>(&fe_to_biguint(&result), NUM_LIMBS, LIMB_BITS));

        println!("compute public input {:?}", public_inputs);

        vec![public_inputs]
    }

    fn synthesize_sub(
        &self,
        config: &Self::Config,
        _challenges: &Challenges<Value<F>>,
        layouter: &mut impl Layouter<F>,
    ) -> Result<(), Error> {

        println!("--------begin assign--------");
        let result_limbs = layouter.assign_region(
            || "assign blob circuit", 
            |mut region| {

                let fp_chip = FpConfig::<F, Fp>::construct(
                    config.fp_config.range.clone(),
                    config.limb_bits,
                    config.num_limbs,
                    modulus::<Fp>(),
                );
                let mut ctx = fp_chip.new_context(region);
                
                let result = self.assign(&mut ctx, &fp_chip, _challenges);


                fp_chip.finalize(&mut ctx);

                ctx.print_stats(&["blobCircuit: FpConfig context"]);

                result
            },
        )?;
        // for (i, v) in result_limbs.iter().enumerate() {
        //     layouter.constrain_instance(v.cell(), config.instance, i)?;
        // }
        
        println!("finish assign");
        Ok(())
    }
}

const MAX_BLOB_DATA_SIZE: usize = 4096 * 31 - 4;

pub fn block_to_blob<F: Field>(block: &Block<F>) -> Result<Vec<u8>, String> {
    // get data from block.txs.rlp_signed
    let data: Vec<u8> = block
        .txs
        .iter()
        .flat_map(|tx| &tx.rlp_signed)
        .cloned()
        .collect();

    if data.len() > MAX_BLOB_DATA_SIZE {
        return Err(format!("data is too large for blob. len={}", data.len()));
    }

    let mut result:Vec<u8> = vec![];

    result.push(0);
    result.extend_from_slice(&(data.len() as u32).to_le_bytes());
    let offset = std::cmp::min(27, data.len());
    result.extend_from_slice(&data[..offset]);

    if data.len() <= 27 {
        for _ in 0..(27 - data.len()) {
            result.push(0);
        }
        return Ok(result);
    }
    
    for chunk in data[27..].chunks(31) {
        let len = std::cmp::min(31, chunk.len());
        result.push(0);
        result.extend_from_slice(&chunk[..len]);
        for _ in 0..(31 - len) {
            result.push(0);
        }
    }

    Ok(result)
}
