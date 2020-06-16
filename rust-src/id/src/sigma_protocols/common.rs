use crypto_common::*;
use curve_arithmetic::*;
use random_oracle::*;

use std::marker::PhantomData;

#[cfg(test)]
pub fn generate_challenge_prefix<R: rand::Rng>(csprng: &mut R) -> Vec<u8> {
    // length of the challenge
    let l = csprng.gen_range(0, 1000);
    let mut challenge_prefix = vec![0; l];
    for v in challenge_prefix.iter_mut() {
        *v = csprng.gen();
    }
    challenge_prefix
}

/// Type of challenges computed from the random oracle.
pub type Challenge<C> = <C as Curve>::Scalar;

/// Sigma protocol types used by the prover and verifier.
pub trait SigmaProtocolTypes {
    /// Prover's commit message.
    type CommitMessage: Serial;
    /// Public input. This includes public input for
    /// the prover and verifier.
    type PublicInput: Serial;
    /// Prover's witness.
    type Witness: Serialize;

    /// Return public input to this session.
    fn public(&self) -> Self::PublicInput;
}

/// An abstraction of a prover in sigma protocols.
pub trait SigmaProtocolProver<C: Curve>: SigmaProtocolTypes {
    /// Prover's state after the first message.
    type ProverState;

    /// First message generated by the prover. We allow this function
    /// to return 'None' if the inputs are inconsistent.
    /// The arguments are
    /// - self -- the prover's public and secret data
    /// - csprng -- a cryptographically secure random number generator
    fn commit_point<R: rand::Rng>(
        &self,
        csprng: &mut R,
    ) -> Option<(Self::CommitMessage, Self::ProverState)>;

    /// Third message generated by the prover. We allow this function to return
    /// 'None' if the inputs are inconsistent.
    /// - self -- the prover's public and secret data
    /// - challenge -- computed challenge
    /// This function is pure and deterministic.
    fn generate_witness(
        &self,
        state: Self::ProverState,
        challenge: &Challenge<C>,
    ) -> Option<Self::Witness>;
}

pub trait SigmaProtocolVerifier<C: Curve>: SigmaProtocolTypes {
    /// Combine the public outputs from the third message with the challenge
    /// and produce a commitment for verification.
    /// - challenge -- computed challenge
    /// - self -- the verifier's public data
    /// This function is pure and deterministic.
    /// It is allowed to return 'None' if some of the input data is malformed,
    /// e.g., vectors of inconsistent lengths.
    fn extract_point(
        &self,
        challenge: &Challenge<C>,
        witness: &Self::Witness,
    ) -> Option<Self::CommitMessage>;
}

#[derive(Debug, Serialize)]
/// Generic structure to contain a single sigma proof.
pub struct SigmaProof<C: Curve, D: SigmaProtocolTypes> {
    pub challenge: Challenge<C>,
    pub witness:   D::Witness,
}

#[derive(Serialize)]
pub struct AndWitness<W1: Serialize, W2: Serialize> {
    pub w1: W1,
    pub w2: W2,
}

/// An adapter to combine multiple provers or multiple verifiers.
/// The marker type C is for convenience in use with the
/// SigmaProtocolProver/Verifier traits below.
pub struct AndAdapter<C: Curve, P1, P2> {
    first:    P1,
    second:   P2,
    _phantom: PhantomData<C>,
}

impl<C: Curve, P1: SigmaProtocolTypes, P2: SigmaProtocolTypes> SigmaProtocolTypes
    for AndAdapter<C, P1, P2>
{
    type CommitMessage = (P1::CommitMessage, P2::CommitMessage);
    type PublicInput = (P1::PublicInput, P2::PublicInput);
    type Witness = AndWitness<P1::Witness, P2::Witness>;

    fn public(&self) -> Self::PublicInput { (self.first.public(), self.second.public()) }
}

impl<C: Curve, P1: SigmaProtocolProver<C>, P2: SigmaProtocolProver<C>> SigmaProtocolProver<C>
    for AndAdapter<C, P1, P2>
{
    type ProverState = (P1::ProverState, P2::ProverState);

    fn commit_point<R: rand::Rng>(
        &self,
        csprng: &mut R,
    ) -> Option<(Self::CommitMessage, Self::ProverState)> {
        let (m1, s1) = self.first.commit_point(csprng)?;
        let (m2, s2) = self.second.commit_point(csprng)?;
        Some(((m1, m2), (s1, s2)))
    }

    fn generate_witness(
        &self,
        state: Self::ProverState,
        challenge: &Challenge<C>,
    ) -> Option<Self::Witness> {
        let w1 = self.first.generate_witness(state.0, challenge)?;
        let w2 = self.second.generate_witness(state.1, challenge)?;
        Some(AndWitness { w1, w2 })
    }
}

impl<C: Curve, P1: SigmaProtocolVerifier<C>, P2: SigmaProtocolVerifier<C>> SigmaProtocolVerifier<C>
    for AndAdapter<C, P1, P2>
{
    fn extract_point(
        &self,
        challenge: &Challenge<C>,
        witness: &Self::Witness,
    ) -> Option<Self::CommitMessage> {
        let p1 = self.first.extract_point(challenge, &witness.w1)?;
        let p2 = self.second.extract_point(challenge, &witness.w2)?;
        Some((p1, p2))
    }
}

impl<C: Curve, P1: SigmaProtocolProver<C>, P2: SigmaProtocolProver<C>> AndAdapter<C, P1, P2> {
    // Extend the current adapter with a new prover.
    pub fn extend_prover<P3: SigmaProtocolProver<C>>(
        self,
        additional_prover: P3,
    ) -> AndAdapter<C, AndAdapter<C, P1, P2>, P3> {
        AndAdapter {
            first:    self,
            second:   additional_prover,
            _phantom: std::default::Default::default(),
        }
    }

    // Extend the current adapter with a new verifier.
    pub fn extend_verifier<P3: SigmaProtocolVerifier<C>>(
        self,
        additional_verifier: P3,
    ) -> AndAdapter<C, AndAdapter<C, P1, P2>, P3> {
        AndAdapter {
            first:    self,
            second:   additional_verifier,
            _phantom: std::default::Default::default(),
        }
    }
}

/// Given a sigma protocol prover and a context (in the form of the random
/// oracle), produce a sigma proof. This function can return 'None' if the input
/// data is inconsistent.
pub fn produce_sigma_proof<R: rand::Rng, C: Curve, D: SigmaProtocolProver<C>>(
    ro: RandomOracle,
    prover: &D,
    csprng: &mut R,
) -> Option<SigmaProof<C, D>> {
    let (point, rands) = prover.commit_point(csprng)?;
    let challenge = ro.append(&prover.public()).finish_to_scalar::<C, _>(&point);
    let witness = prover.generate_witness(rands, &challenge)?;
    Some(SigmaProof { challenge, witness })
}

/// Verify a single sigma proof, given a sigma proof verifier and a context in
/// the form of an instantiated random oracle.
pub fn verify_sigma_proof<C: Curve, D: SigmaProtocolVerifier<C>>(
    ro: RandomOracle,
    verifier: &D,
    proof: &SigmaProof<C, D>,
) -> bool {
    match verifier.extract_point(&proof.challenge, &proof.witness) {
        None => false,
        Some(ref point) => {
            let computed_challenge = ro
                .append(&verifier.public())
                .finish_to_scalar::<C, _>(point);
            computed_challenge == proof.challenge
        }
    }
}
