//! This module implements `Chunk` related data types.
//! A chunk is a list of blocks.
use eth_types::{ToBigEndian, H256, U256, ToLittleEndian};
use ethers_core::utils::{keccak256, rlp::Encodable};
use halo2_proofs::halo2curves::bn256::Fr;
use bls12_381::Scalar as Fp;
use serde::{Deserialize, Serialize};
use snark_verifier::loader::halo2::halo2_ecc::halo2_base::utils::{decompose_biguint, fe_to_biguint};
use std::iter;
use zkevm_circuits::witness::Block;

#[derive(Default, Debug, Clone, Copy, Deserialize, Serialize)]
/// A chunk is a set of continuous blocks.
/// A ChunkHash consists of 4 hashes, representing the changes incurred by this chunk of blocks:
/// - state root before this chunk
/// - state root after this chunk
/// - the withdraw root after this chunk
/// - the data hash of this chunk
/// - if the chunk is padded (en empty but valid chunk that is padded for aggregation)
pub struct ChunkHash {
    /// Chain identifier
    pub chain_id: u64,
    /// state root before this chunk
    pub prev_state_root: H256,
    /// state root after this chunk
    pub post_state_root: H256,
    /// the withdraw root after this chunk
    pub withdraw_root: H256,
    /// the data hash of this chunk
    pub data_hash: H256,
    // bls challenge point
    pub challenge_point: H256,
    // bls partial result
    pub partial_result: H256,
    /// if the chunk is a padded chunk
    pub is_padding: bool,
}

impl ChunkHash {
    /// Construct by a witness block.
    pub fn from_witness_block(block: &Block<Fr>, is_padding: bool) -> Self {
        // <https://github.com/scroll-tech/zkevm-circuits/blob/25dd32aa316ec842ffe79bb8efe9f05f86edc33e/bus-mapping/src/circuit_input_builder.rs#L690>

        let mut total_l1_popped = block.start_l1_queue_index;
        log::debug!("chunk-hash: start_l1_queue_index = {}", total_l1_popped);
        let data_bytes = iter::empty()
            // .chain(block_headers.iter().flat_map(|(&block_num, block)| {
            .chain(block.context.ctxs.iter().flat_map(|(b_num, b_ctx)| {
                let num_l2_txs = block
                    .txs
                    .iter()
                    .filter(|tx| !tx.tx_type.is_l1_msg() && tx.block_number == *b_num)
                    .count() as u64;
                let num_l1_msgs = block
                    .txs
                    .iter()
                    .filter(|tx| tx.tx_type.is_l1_msg() && tx.block_number == *b_num)
                    // tx.nonce alias for queue_index for l1 msg tx
                    .map(|tx| tx.nonce)
                    .max()
                    .map_or(0, |max_queue_index| max_queue_index - total_l1_popped + 1);
                total_l1_popped += num_l1_msgs;

                let num_txs = (num_l2_txs + num_l1_msgs) as u16;
                log::debug!(
                    "chunk-hash: [block {}] total_l1_popped = {}, num_l1_msgs = {}, num_l2_txs = {}, num_txs = {}",
                    b_num,
                    total_l1_popped,
                    num_l1_msgs,
                    num_l2_txs,
                    num_txs,
                );

                iter::empty()
                    // Block Values
                    .chain(b_ctx.number.as_u64().to_be_bytes())
                    .chain(b_ctx.timestamp.as_u64().to_be_bytes())
                    .chain(b_ctx.base_fee.to_be_bytes())
                    .chain(b_ctx.gas_limit.to_be_bytes())
                    .chain(num_txs.to_be_bytes())
            }))
            // Tx Hashes
            .chain(block.txs.iter().flat_map(|tx| tx.hash.to_fixed_bytes()))
            .collect::<Vec<u8>>();

        let data_hash = H256(keccak256(data_bytes));
        log::debug!(
            "chunk-hash: data hash = {}",
            hex::encode(data_hash.to_fixed_bytes())
        );

        let post_state_root = block
            .context
            .ctxs
            .last_key_value()
            .map(|(_, b_ctx)| b_ctx.eth_block.state_root)
            .unwrap_or(H256(block.prev_state_root.to_be_bytes()));

        //TODO:compute partial_result from witness block;
        // let omega = Fp::from(123).pow(&[(FP_S - 12) as u64, 0, 0, 0]);

        // let partial_result = polyeval()

        Self {
            chain_id: block.chain_id,
            prev_state_root: H256(block.prev_state_root.to_be_bytes()),
            post_state_root,
            withdraw_root: H256(block.withdraw_root.to_be_bytes()),
            data_hash,
            challenge_point: H256(block.challenge_point.to_be_bytes()),
            partial_result: H256(block.partial_result.to_be_bytes()),
            is_padding,
        }
    }

    /// Sample a chunk hash from random (for testing)
    #[cfg(test)]
    pub(crate) fn mock_random_chunk_hash_for_testing<R: rand::RngCore>(r: &mut R) -> Self {
        let mut prev_state_root = [0u8; 32];
        r.fill_bytes(&mut prev_state_root);
        let mut post_state_root = [0u8; 32];
        r.fill_bytes(&mut post_state_root);
        let mut withdraw_root = [0u8; 32];
        r.fill_bytes(&mut withdraw_root);
        let mut data_hash = [0u8; 32];
        r.fill_bytes(&mut data_hash);
        let mut buf = [0u8; 64];
        r.fill_bytes(&mut buf);
        let mut challenge_point = Fp::from_bytes_wide(&buf).to_bytes();
        // println!("random cp le bytes{:?}", challenge_point);
        // println!("random cp{}", Fp::from_bytes_wide(&buf));
        let mut buf1 = [0u8; 64];
        r.fill_bytes(&mut buf1);
        let mut partial_result = Fp::from_bytes_wide(&buf1).to_bytes();
        // r.fill_bytes(&mut partial_result);
        Self {
            chain_id: 0,
            prev_state_root: prev_state_root.into(),
            post_state_root: post_state_root.into(),
            withdraw_root: withdraw_root.into(),
            data_hash: data_hash.into(),
            challenge_point: challenge_point.into(),
            partial_result: partial_result.into(),
            is_padding: false,
        }
    }

    /// Build a padded chunk from previous one
    #[cfg(test)]
    pub(crate) fn mock_padded_chunk_hash_for_testing(previous_chunk: &Self) -> Self {
        assert!(
            !previous_chunk.is_padding,
            "previous chunk is padded already"
        );
        Self {
            chain_id: previous_chunk.chain_id,
            prev_state_root: previous_chunk.prev_state_root,
            post_state_root: previous_chunk.post_state_root,
            withdraw_root: previous_chunk.withdraw_root,
            data_hash: previous_chunk.data_hash,
            challenge_point: previous_chunk.challenge_point,
            partial_result: previous_chunk.partial_result,
            is_padding: true,
        }
    }

    /// Public input hash for a given chunk is defined as
    ///  keccak( chain id || prev state root || post state root || withdraw root || data hash )
    pub fn public_input_hash(&self) -> H256 {
        let preimage = self.extract_hash_preimage();
        keccak256::<&[u8]>(preimage.as_ref()).into()
    }

    /// Extract the preimage for the hash
    ///  chain id || prev state root || post state root || withdraw root || data hash
    pub fn extract_hash_preimage(&self) -> Vec<u8> {
        [
            self.chain_id.to_be_bytes().as_ref(),
            self.prev_state_root.as_bytes(),
            self.post_state_root.as_bytes(),
            self.withdraw_root.as_bytes(),
            self.data_hash.as_bytes(),
        ]
        .concat()
    }

    /// decompose challenge_point
    pub fn challenge_point(&self) -> Vec<Fr>{
        let cp_fe = Fp::from_bytes(&self.challenge_point.into()).unwrap();
        // println!("cp le bytes{:?}", self.challenge_point);
        // println!("cpfe{}", cp_fe);
        decompose_biguint::<Fr>(&fe_to_biguint(&cp_fe), 3, 88)

    }

    /// decompose partial_result
    pub fn partial_result(&self) -> Vec<Fr>{
        let pr_fe = Fp::from_bytes(&self.partial_result.into()).unwrap();
        // println!("prfe{}", pr_fe);
        decompose_biguint::<Fr>(&fe_to_biguint(&pr_fe), 3, 88)
    }

}
