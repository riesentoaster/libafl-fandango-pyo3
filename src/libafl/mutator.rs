use std::borrow::Cow;

use libafl::{
    Error,
    corpus::CorpusId,
    inputs::BytesInput,
    mutators::{MutationResult, Mutator},
};
use libafl_bolts::Named;

use crate::fandango::FandangoClient;

pub struct FandangoPseudoMutator<F> {
    fandango: F,
}

impl<F> FandangoPseudoMutator<F> {
    pub fn new(fandango: F) -> Self {
        Self { fandango }
    }
}

impl<F: FandangoClient, S> Mutator<BytesInput, S> for FandangoPseudoMutator<F> {
    fn mutate(&mut self, _state: &mut S, input: &mut BytesInput) -> Result<MutationResult, Error> {
        let new_input = self
            .fandango
            .next_input()
            .map_err(|e| Error::illegal_state(e.to_string()))?;
        *input = BytesInput::new(new_input);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

impl<F> Named for FandangoPseudoMutator<F> {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("FandangoPseudoMutator")
    }
}
