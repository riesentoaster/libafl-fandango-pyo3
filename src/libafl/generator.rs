use libafl::{Error, generators::Generator, inputs::BytesInput};

use crate::fandango::FandangoClient;

pub struct FandangoGenerator<F> {
    fandango: F,
}

impl<F> FandangoGenerator<F> {
    pub fn new(fandango: F) -> Self {
        Self { fandango }
    }
}

impl<F: FandangoClient, S> Generator<BytesInput, S> for FandangoGenerator<F> {
    fn generate(&mut self, _state: &mut S) -> Result<BytesInput, Error> {
        let input = self
            .fandango
            .next_input()
            .map_err(|e| Error::illegal_state(format!("Fandango error: {e}")))?;
        Ok(input.into())
    }
}
