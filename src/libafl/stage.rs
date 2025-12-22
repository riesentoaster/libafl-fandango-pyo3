use std::borrow::Cow;

use libafl::{
    Error, Evaluator, HasNamedMetadata,
    corpus::HasCurrentCorpusId,
    inputs::{BytesInput, ValueInput},
    mutators::{MutationResult, Mutator},
    stages::{Restartable, RetryCountRestartHelper, Stage},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand as _};

use crate::fandango::FandangoPythonModule;

pub struct FandangoPostMutationalStage<M> {
    fandango: FandangoPythonModule,
    mutators: M,
    min_iterations: usize,
    max_iterations: usize,
}

impl<M> FandangoPostMutationalStage<M> {
    /// Create a new FandangoPostMutationalStage
    ///
    /// # Arguments
    ///
    /// * `fandango` - The FandangoPythonModule to use
    /// * `mutators` - The mutators to use
    /// * `min_iterations` - The minimum number of iterations to run for each generated input (inclusive)
    /// * `max_iterations` - The maximum number of iterations to run for each generated input (inclusive)
    pub fn new(
        fandango: FandangoPythonModule,
        mutators: M,
        min_iterations: usize,
        max_iterations: usize,
    ) -> Self {
        Self {
            fandango,
            mutators,
            min_iterations,
            max_iterations,
        }
    }
}

impl<E, EM, M, S, Z> Stage<E, EM, S, Z> for FandangoPostMutationalStage<M>
where
    Z: Evaluator<E, EM, BytesInput, S>,
    M: Mutator<BytesInput, S>,
    S: HasRand,
{
    fn perform(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        state: &mut S,
        manager: &mut EM,
    ) -> Result<(), libafl::Error> {
        let input: ValueInput<Vec<u8>> = self
            .fandango
            .next_input()
            .map_err(|e| Error::illegal_state(e.to_string()))?
            .into();

        let iterations = 1 + state
            .rand_mut()
            .between(self.min_iterations, self.max_iterations);

        // Run the unchanged input
        let (_, corpus_id) = fuzzer.evaluate_filtered(state, executor, manager, &input)?;
        self.mutators.post_exec(state, corpus_id)?;

        for _ in 0..iterations {
            let mut cloned_input = input.clone();
            let mutation_result = self.mutators.mutate(state, &mut cloned_input)?;
            if matches!(mutation_result, MutationResult::Skipped) {
                continue;
            }

            let (_, corpus_id) =
                fuzzer.evaluate_filtered(state, executor, manager, &cloned_input)?;
            self.mutators.post_exec(state, corpus_id)?;
        }

        Ok(())
    }
}

impl<M> Named for FandangoPostMutationalStage<M> {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("FandangoPostMutationalStage")
    }
}

impl<M, S> Restartable<S> for FandangoPostMutationalStage<M>
where
    S: HasNamedMetadata + HasCurrentCorpusId,
{
    fn should_restart(&mut self, state: &mut S) -> Result<bool, Error> {
        RetryCountRestartHelper::should_restart(state, self.name(), 3)
    }

    fn clear_progress(&mut self, state: &mut S) -> Result<(), Error> {
        RetryCountRestartHelper::clear_progress(state, self.name())
    }
}
