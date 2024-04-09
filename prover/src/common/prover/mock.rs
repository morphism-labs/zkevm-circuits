use super::Prover;
use crate::utils::read_env_var;
use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};
use once_cell::sync::Lazy;
use snark_verifier_sdk::CircuitExt;

pub static MOCK_PROVE: Lazy<bool> = Lazy::new(|| read_env_var("MOCK_PROVE", true));

impl Prover {
    pub fn assert_if_mock_prover<C: CircuitExt<Fr>>(id: &str, degree: u32, circuit: &C) {
        if !*MOCK_PROVE {
            return;
        }

        log::info!("Mock prove for {id} - BEGIN");

        let instances = circuit.instances();
        let mock_prover = MockProver::<Fr>::run(degree, circuit, instances).unwrap();

        mock_prover.assert_satisfied_par();

        log::info!("Mock prove for {id} - END");
    }
}
