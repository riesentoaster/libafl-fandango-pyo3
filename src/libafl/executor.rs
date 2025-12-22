use libafl::{
    Error,
    executors::{Executor, ExitKind, HasObservers},
    inputs::{BytesInput, HasTargetBytes as _},
    observers::RefCellValueObserver,
};
use libafl_bolts::tuples::{Handle, MatchNameRef, RefIndexable};

use crate::fandango::FandangoPythonModule;

pub struct FandangoParseExecutor<'a, OT> {
    fandango: FandangoPythonModule,
    num_parses_observer: Handle<RefCellValueObserver<'a, u32>>,
    observers: OT,
}

impl<'a, OT> FandangoParseExecutor<'a, OT> {
    pub fn new(
        fandango: FandangoPythonModule,
        num_parses_observer: Handle<RefCellValueObserver<'a, u32>>,
        observers: OT,
    ) -> Self {
        Self {
            fandango,
            num_parses_observer,
            observers,
        }
    }
}

impl<'a, EM, OT, S, Z> Executor<EM, BytesInput, S, Z> for FandangoParseExecutor<'a, OT>
where
    OT: MatchNameRef,
{
    fn run_target(
        &mut self,
        _fuzzer: &mut Z,
        _state: &mut S,
        _mgr: &mut EM,
        input: &BytesInput,
    ) -> Result<libafl::executors::ExitKind, Error> {
        let num_parses = self
            .fandango
            .parse_input(&input.target_bytes())
            .map_err(|e| Error::illegal_state(e.to_string()))?;

        self.observers
            .get_mut(&self.num_parses_observer)
            .ok_or(Error::illegal_state(
                "num_parses_observer not found".to_string(),
            ))?
            .set(num_parses);
        Ok(ExitKind::Ok)
    }
}

impl<'a, OT> HasObservers for FandangoParseExecutor<'a, OT>
where
    OT: MatchNameRef,
{
    type Observers = OT;

    fn observers(&self) -> RefIndexable<&Self::Observers, Self::Observers> {
        RefIndexable::from(&self.observers)
    }

    fn observers_mut(&mut self) -> RefIndexable<&mut Self::Observers, Self::Observers> {
        RefIndexable::from(&mut self.observers)
    }
}
