use std::borrow::Cow;

use libafl::{
    Error,
    corpus::CorpusId,
    inputs::BytesInput,
    mutators::{MutationResult, Mutator},
};
use libafl_bolts::Named;

use crate::fandango::FandangoPythonModule;

pub struct FandangoPseudoMutator {
    fandango: FandangoPythonModule,
}

impl FandangoPseudoMutator {
    pub fn new(fandango: FandangoPythonModule) -> Self {
        Self { fandango }
    }
}

impl<S> Mutator<BytesInput, S> for FandangoPseudoMutator {
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

impl Named for FandangoPseudoMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("FandangoPseudoMutator")
    }
}
