use libafl::{Error, generators::Generator, inputs::BytesInput};

use crate::fandango::FandangoPythonModule;

pub struct FandangoGenerator {
    fandango: FandangoPythonModule,
}

impl FandangoGenerator {
    pub fn new(fandango: FandangoPythonModule) -> Self {
        Self { fandango }
    }
}

impl<S> Generator<BytesInput, S> for FandangoGenerator {
    fn generate(&mut self, _state: &mut S) -> Result<BytesInput, Error> {
        let input = self
            .fandango
            .next_input()
            .map_err(|e| Error::illegal_state(e.to_string()))?;
        Ok(input.into())
    }
}
