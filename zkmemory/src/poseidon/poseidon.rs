extern crate alloc;

use alloc::{fmt, format, string::String, vec::Vec};
use core::{fmt::Debug, iter, marker::PhantomData};
use ff::{Field, PrimeField};
use halo2_proofs::{
    circuit::{AssignedCell, Cell, Chip, Layouter, Value},
    plonk::Error,
};

extern crate std;

/// Trait for a variable in the circuit.
pub trait Var<F: Field>: Clone + Debug + From<AssignedCell<F, F>> {
    /// The cell at which this variable was allocated.
    fn cell(&self) -> Cell;

    /// The value allocated to this variable.
    fn value(&self) -> Value<F>;
}

impl<F: Field> Var<F> for AssignedCell<F, F> {
    fn cell(&self) -> Cell {
        self.cell()
    }

    fn value(&self) -> Value<F> {
        self.value().cloned()
    }
}

/// A word in the Poseidon state.
#[derive(Clone, Debug)]
pub struct StateWord<F: Field + PrimeField>(pub AssignedCell<F, F>);

impl<F: Field + PrimeField> From<StateWord<F>> for AssignedCell<F, F> {
    fn from(state_word: StateWord<F>) -> AssignedCell<F, F> {
        state_word.0
    }
}

impl<F: Field + PrimeField> From<AssignedCell<F, F>> for StateWord<F> {
    fn from(cell_value: AssignedCell<F, F>) -> StateWord<F> {
        StateWord(cell_value)
    }
}

impl<F: Field + PrimeField> Var<F> for StateWord<F> {
    fn cell(&self) -> Cell {
        self.0.cell()
    }

    fn value(&self) -> Value<F> {
        self.0.value().cloned()
    }
}

pub trait Spec<F: Field + PrimeField, const T: usize, const R: usize>: fmt::Debug {
    /// The number of full rounds for this specification.
    ///
    /// This must be an even number.
    fn full_rounds() -> usize;

    /// The number of partial rounds for this specification.
    fn partial_rounds() -> usize;

    /// The S-box for this specification.
    fn sbox(val: F) -> F;

    /// Side-loaded index of the first correct and secure MDS that will be generated by
    /// the reference implementation.
    ///
    /// This is used by the default implementation of [`Spec::constants`]. If you are
    /// hard-coding the constants, you may leave this unimplemented.
    fn secure_mds() -> usize;

    /// Generates `(round_constants, mds, mds^-1)` corresponding to this specification.
    fn constants() -> (Vec<[F; T]>, [[F; T]; T], [[F; T]; T]);
}

pub trait Domain<F: Field + PrimeField, const R: usize> {
    /// Iterator that outputs padding field elements.
    type Padding: IntoIterator<Item = F>;

    /// The name of this domain, for debug formatting purposes.
    fn name() -> String;

    /// The initial capacity element, encoding this domain.
    fn initial_capacity_element() -> F;

    /// Returns the padding to be appended to the input.
    fn padding(input_len: usize) -> Self::Padding;
}

pub struct ConstantLength<const L: usize>;

impl<F: PrimeField, const R: usize, const L: usize> Domain<F, R> for ConstantLength<L> {
    type Padding = iter::Take<iter::Repeat<F>>;

    fn name() -> String {
        format!("ConstantLength<{}>", L)
    }

    fn initial_capacity_element() -> F {
        // Capacity value is $length \cdot 2^64 + (o-1)$ where o is the output length.
        // We hard-code an output length of 1.
        F::from_u128((L as u128) << 64)
    }

    fn padding(input_len: usize) -> Self::Padding {
        assert_eq!(input_len, L);
        // For constant-input-length hashing, we pad the input with zeroes to a multiple
        // of R. On its own this would not be sponge-compliant padding, but the
        // Poseidon authors encode the constant length into the capacity element, ensuring
        // that inputs of different lengths do not share the same permutation.
        let k = (L + R - 1) / R;
        iter::repeat(F::ZERO).take(k * R - L)
    }
}

/// The state of the `Sponge`.
pub trait SpongeMode {}
impl<F, const R: usize> SpongeMode for Absorbing<F, R> {}
impl<F, const R: usize> SpongeMode for Squeezing<F, R> {}

impl<F: fmt::Debug, const R: usize> Absorbing<F, R> {
    pub(crate) fn init_with(val: F) -> Self {
        Self(
            iter::once(Some(val))
                .chain((1..R).map(|_| None))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )
    }
}

/// The absorbing state of the `Sponge`.
#[derive(Debug)]
pub struct Absorbing<F, const R: usize>(pub(crate) SpongeRate<F, R>);

/// The squeezing state of the `Sponge`.
#[derive(Debug)]
pub struct Squeezing<F, const R: usize>(pub(crate) SpongeRate<F, R>);

/// The type used to hold permutation state.
pub(crate) type State<F, const T: usize> = [F; T];

/// The type used to hold sponge rate.
pub(crate) type SpongeRate<F, const R: usize> = [Option<F>; R];

/// A word from the padded input to a Poseidon sponge.
#[derive(Clone, Debug)]
pub enum PaddedWord<F: Field + PrimeField> {
    /// A message word provided by the prover.
    Message(AssignedCell<F, F>),
    /// A padding word, that will be fixed in the circuit parameters.
    Padding(F),
}

/// The set of circuit instructions required to use the Poseidon permutation.
pub trait PoseidonInstructions<
    F: Field + PrimeField,
    S: Spec<F, T, R>,
    const T: usize,
    const R: usize,
>: Chip<F>
{
    /// Variable representing the word over which the Poseidon permutation operates.
    type Word: Clone + fmt::Debug + From<AssignedCell<F, F>> + Into<AssignedCell<F, F>>;

    /// Applies the Poseidon permutation to the given state.
    fn permute(
        &self,
        layouter: &mut impl Layouter<F>,
        initial_state: &State<Self::Word, T>,
    ) -> Result<State<Self::Word, T>, Error>;
}

/// The set of circuit instructions required to use the [`Sponge`] and [`Hash`] gadgets.
///
/// [`Hash`]: self::Hash
pub trait PoseidonSpongeInstructions<
    F: Field + PrimeField,
    S: Spec<F, T, R>,
    D: Domain<F, R>,
    const T: usize,
    const R: usize,
>: PoseidonInstructions<F, S, T, R>
{
    /// Returns the initial empty state for the given domain.
    fn initial_state(&self, layouter: &mut impl Layouter<F>)
        -> Result<State<Self::Word, T>, Error>;

    /// Adds the given input to the state.
    fn add_input(
        &self,
        layouter: &mut impl Layouter<F>,
        initial_state: &State<Self::Word, T>,
        input: &Absorbing<PaddedWord<F>, R>,
    ) -> Result<State<Self::Word, T>, Error>;

    /// Extracts sponge output from the given state.
    fn get_output(state: &State<Self::Word, T>) -> Squeezing<Self::Word, R>;
}

/// A Poseidon sponge.
pub(crate) struct Sponge<
    F: Field + PrimeField,
    S: Spec<F, T, RATE>,
    M: SpongeMode,
    const T: usize,
    const RATE: usize,
> {
    mode: M,
    state: State<F, T>,
    mds_matrix: [[F; T]; T],
    round_constants: Vec<[F; T]>,
    _marker: PhantomData<S>,
}

impl<F: Field + PrimeField, S: Spec<F, T, RATE>, const T: usize, const RATE: usize>
    Sponge<F, S, Absorbing<F, RATE>, T, RATE>
{
    /// Constructs a new sponge for the given Poseidon specification.
    pub(crate) fn new(initial_capacity_element: F) -> Self {
        let (round_constants, mds_matrix, _) = S::constants();

        let mode = Absorbing([None; RATE]);
        let mut state = [F::ZERO; T];
        state[RATE] = initial_capacity_element;

        Sponge {
            mode,
            state,
            mds_matrix,
            round_constants,
            _marker: PhantomData::default(),
        }
    }

    /// Absorbs an element into the sponge.
    pub(crate) fn absorb(&mut self, value: F) {
        for entry in self.mode.0.iter_mut() {
            if entry.is_none() {
                *entry = Some(value);
                return;
            }
        }

        // We've already absorbed as many elements as we can
        let _ = poseidon_sponge::<F, S, T, RATE>(
            &mut self.state,
            Some(&self.mode),
            &self.mds_matrix,
            &self.round_constants,
        );
        self.mode = Absorbing::init_with(value);
    }

    /// Transitions the sponge into its squeezing state.
    pub(crate) fn finish_absorbing(mut self) -> Sponge<F, S, Squeezing<F, RATE>, T, RATE> {
        let mode = poseidon_sponge::<F, S, T, RATE>(
            &mut self.state,
            Some(&self.mode),
            &self.mds_matrix,
            &self.round_constants,
        );

        Sponge {
            mode,
            state: self.state,
            mds_matrix: self.mds_matrix,
            round_constants: self.round_constants,
            _marker: PhantomData::default(),
        }
    }
}

fn poseidon_sponge<
    F: Field + PrimeField,
    S: Spec<F, T, RATE>,
    const T: usize,
    const RATE: usize,
>(
    state: &mut State<F, T>,
    input: Option<&Absorbing<F, RATE>>,
    mds_matrix: &[[F; T]; T],
    round_constants: &[[F; T]],
) -> Squeezing<F, RATE> {
    if let Some(Absorbing(input)) = input {
        // `Iterator::zip` short-circuits when one iterator completes, so this will only
        // mutate the rate portion of the state.
        for (word, value) in state.iter_mut().zip(input.iter()) {
            *word += value.expect("poseidon_sponge is called with a padded input");
        }
    }

    permute::<F, S, T, RATE>(state, mds_matrix, round_constants);

    let mut output = [None; RATE];
    for (word, value) in output.iter_mut().zip(state.iter()) {
        *word = Some(*value);
    }
    Squeezing(output)
}

/// Runs the Poseidon permutation on the given state.
pub(crate) fn permute<
    F: Field + PrimeField,
    S: Spec<F, T, RATE>,
    const T: usize,
    const RATE: usize,
>(
    state: &mut State<F, T>,
    mds: &[[F; T]; T],
    round_constants: &[[F; T]],
) {
    let r_f = S::full_rounds() / 2;
    let r_p = S::partial_rounds();

    let apply_mds = |state: &mut State<F, T>| {
        let mut new_state = [F::ZERO; T];
        // Matrix multiplication
        #[allow(clippy::needless_range_loop)]
        for i in 0..T {
            for j in 0..T {
                new_state[i] += mds[i][j] * state[j];
            }
        }
        *state = new_state;
    };

    let full_round = |state: &mut State<F, T>, rcs: &[F; T]| {
        for (word, rc) in state.iter_mut().zip(rcs.iter()) {
            *word = S::sbox(*word + rc);
        }
        apply_mds(state);
    };

    let part_round = |state: &mut State<F, T>, rcs: &[F; T]| {
        for (word, rc) in state.iter_mut().zip(rcs.iter()) {
            *word += rc;
        }
        // In a partial round, the S-box is only applied to the first state word.
        state[0] = S::sbox(state[0]);
        apply_mds(state);
    };

    iter::empty()
        .chain(iter::repeat(&full_round as &dyn Fn(&mut State<F, T>, &[F; T])).take(r_f))
        .chain(iter::repeat(&part_round as &dyn Fn(&mut State<F, T>, &[F; T])).take(r_p))
        .chain(iter::repeat(&full_round as &dyn Fn(&mut State<F, T>, &[F; T])).take(r_f))
        .zip(round_constants.iter())
        .fold(state, |state, (round, rcs)| {
            round(state, rcs);
            state
        });
}

impl<F: Field + PrimeField, S: Spec<F, T, RATE>, const T: usize, const RATE: usize>
    Sponge<F, S, Squeezing<F, RATE>, T, RATE>
{
    /// Squeezes an element from the sponge.
    pub(crate) fn squeeze(&mut self) -> F {
        loop {
            for entry in self.mode.0.iter_mut() {
                if let Some(e) = entry.take() {
                    return e;
                }
            }

            // We've already squeezed out all available elements
            self.mode = poseidon_sponge::<F, S, T, RATE>(
                &mut self.state,
                None,
                &self.mds_matrix,
                &self.round_constants,
            );
        }
    }
}

/// A Poseidon hash function, built around a sponge.
pub struct HashTest<
    F: Field + PrimeField,
    S: Spec<F, T, RATE>,
    D: Domain<F, RATE>,
    const T: usize,
    const RATE: usize,
> {
    sponge: Sponge<F, S, Absorbing<F, RATE>, T, RATE>,
    _domain: PhantomData<D>,
}

impl<
        F: Field + PrimeField,
        S: Spec<F, T, RATE>,
        D: Domain<F, RATE>,
        const T: usize,
        const RATE: usize,
    > HashTest<F, S, D, T, RATE>
{
    /// Initializes a new hasher.
    pub fn init() -> Self {
        HashTest {
            sponge: Sponge::new(D::initial_capacity_element()),
            _domain: PhantomData::default(),
        }
    }
}

impl<
        F: Field + PrimeField,
        S: Spec<F, T, RATE>,
        const T: usize,
        const RATE: usize,
        const L: usize,
    > HashTest<F, S, ConstantLength<L>, T, RATE>
{
    /// Hashes the given input.
    pub fn hash(mut self, message: [F; L]) -> F {
        for value in message
            .into_iter()
            .chain(<ConstantLength<L> as Domain<F, RATE>>::padding(L))
        {
            self.sponge.absorb(value);
        }
        self.sponge.finish_absorbing().squeeze()
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use crate::poseidon::poseidon_constants::*;
    use alloc::vec::Vec;
    use ff::{Field, PrimeField};
    use halo2curves::pasta::Fp;

    ///
    #[derive(Debug)]
    pub struct OrchardNullifier;

    impl Spec<Fp, 3, 2> for OrchardNullifier {
        fn full_rounds() -> usize {
            8
        }

        fn partial_rounds() -> usize {
            56
        }

        fn sbox(val: Fp) -> Fp {
            val.pow_vartime([5])
        }

        fn secure_mds() -> usize {
            unimplemented!()
        }
        fn constants() -> (Vec<[Fp; 3]>, [[Fp; 3]; 3], [[Fp; 3]; 3]) {
            (ROUND_CONSTANTS[..].to_vec(), MDS, MDS_INV)
        }
    }

    use halo2curves::pasta::pallas::Base;
    #[test]
    fn poseidon_hash() {
        let message = [Base::from(6), Base::from(42)];

        let (round_constants, mds, _) = OrchardNullifier::constants();

        let hasher = HashTest::<_, OrchardNullifier, ConstantLength<2>, 3, 2>::init();
        let result = hasher.hash(message);

        // The result should be equivalent to just directly applying the permutation and
        // taking the first state element as the output.
        let mut state = [message[0], message[1], Base::from_u128(2 << 64)];
        permute::<_, OrchardNullifier, 3, 2>(&mut state, &mds, &round_constants);
        assert_eq!(state[0], result);
    }
}
