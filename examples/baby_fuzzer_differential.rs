use serde::{Deserialize, Serialize};
use std::{borrow::Cow, cell::RefCell, path::PathBuf};

use clap::Parser;
use libafl::{
    HasMetadata,
    corpus::{Corpus, InMemoryCorpus, OnDiskCorpus, Testcase},
    events::{EventConfig, Launcher},
    executors::{DiffExecutor, ExitKind, InProcessExecutor},
    feedback_or_fast,
    feedbacks::{
        CrashFeedback, DiffExitKindFeedback, DiffFeedback, Feedback, MaxMapFeedback,
        StateInitializer, differential::DiffResult,
    },
    fuzzer::{Fuzzer, StdFuzzer},
    inputs::{BytesInput, HasTargetBytes},
    monitors::MultiMonitor,
    mutators::{HavocScheduledMutator, havoc_mutations},
    observers::{RefCellValueObserver, StdMapObserver},
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasCorpus, StdState},
};
use libafl_bolts::{
    Error, ErrorBacktrace, Named, SerdeAny,
    core_affinity::Cores,
    current_nanos,
    ownedref::OwnedRef,
    rands::StdRand,
    shmem::{ShMemProvider, StdShMemProvider},
    tuples::{Handle, Handled as _, MatchNameRef, tuple_list},
};
use libafl_fandango_pyo3::{
    fandango::{FandangoPythonModule, FandangoPythonModuleInitError},
    libafl::FandangoParseExecutor,
};

#[derive(Parser)]
#[command(name = "run_fandango")]
#[command(about = "Run the fandango interface in Python")]
struct Args {
    #[arg(short, long, default_value = "examples/even_numbers_parse.fan")]
    fandango_file: String,
    #[arg(short, long, value_parser = Cores::from_cmdline, default_value = "all")]
    cores: Cores,
}

pub fn main() -> Result<(), String> {
    env_logger::init();

    let args = Args::parse();

    let monitor = MultiMonitor::new(|s| println!("{s}"));

    let shmem_provider = StdShMemProvider::new().expect("Failed to init shared memory");

    // Generate one Generator to ensure the interpreter is ready
    if let Err(FandangoPythonModuleInitError::PyErr(e)) =
        FandangoPythonModule::new(&args.fandango_file, &[])
    {
        return Err(format!(
            "You may need to set the PYTHONPATH environment variable to the path of the Python interpreter, e.g. `export PYTHONPATH=$(echo .venv/lib/python*/site-packages)`. Underlying error: {:?}",
            e
        ));
    }

    let mut run_client = |state: Option<_>, mut restarting_mgr, _client_description| {
        log::info!("Running client");

        let mut pseudo_coverage_map = vec![0; 9];

        let map_observer = unsafe {
            StdMapObserver::from_mut_ptr(
                "coverage",
                pseudo_coverage_map.as_mut_ptr(),
                pseudo_coverage_map.len(),
            )
        };
        let coverage_feedback = MaxMapFeedback::new(&map_observer);

        let is_divisible_by_2_harness = RefCell::new(false);
        let is_divisible_by_2_observer_harness = RefCellValueObserver::new(
            "is_divisible_by_2_harness",
            OwnedRef::Ref(&is_divisible_by_2_harness),
        );
        let is_divisible_by_2_fandango = RefCell::new(0);
        let is_divisible_by_2_observer_fandango = RefCellValueObserver::new(
            "is_divisible_by_2_fandango",
            OwnedRef::Ref(&is_divisible_by_2_fandango),
        );

        let is_divisible_by_2_diff_feedback = DiffFeedback::new(
            "result_diff_feedback",
            &is_divisible_by_2_observer_harness,
            &is_divisible_by_2_observer_fandango,
            |a: &RefCellValueObserver<'_, bool>, b: &RefCellValueObserver<'_, u32>| {
                let a_ref = *a.get_ref();
                let b_ref = *b.get_ref();
                let b_is_divisible_by_2 = b_ref != 0;
                if a_ref == b_is_divisible_by_2 {
                    DiffResult::Equal
                } else {
                    log::warn!("Diff: {a_ref} != {b_is_divisible_by_2}({b_ref})");
                    DiffResult::Diff
                }
            },
        )?;

        let mut feedback = coverage_feedback;

        let mut objective = feedback_or_fast!(
            LogFeedback::new(
                is_divisible_by_2_observer_fandango.handle(),
                is_divisible_by_2_observer_harness.handle()
            ),
            DiffExitKindFeedback::new(),
            CrashFeedback::new(),
            is_divisible_by_2_diff_feedback
        );

        let mut state = state.unwrap_or_else(|| {
            StdState::new(
                StdRand::with_seed(current_nanos()),
                InMemoryCorpus::new(),
                OnDiskCorpus::new(PathBuf::from("./crashes")).unwrap(),
                &mut feedback,
                &mut objective,
            )
            .unwrap()
        });

        let mut fuzzer = StdFuzzer::new(QueueScheduler::new(), feedback, objective);

        let mut update_coverage = |index: usize| pseudo_coverage_map[index] += 1;

        let mut harness = |input: &BytesInput| {
            update_coverage(0);
            let target = input.target_bytes().to_vec();
            let number = match String::from_utf8(target) {
                Ok(number) => {
                    update_coverage(1);
                    number
                }
                Err(_) => {
                    update_coverage(2);
                    *is_divisible_by_2_harness.borrow_mut() = false;
                    return ExitKind::Ok;
                }
            };
            update_coverage(3);

            let number = match number.parse::<u128>() {
                Ok(number) => {
                    update_coverage(4);
                    number
                }
                Err(_) => {
                    update_coverage(5);
                    *is_divisible_by_2_harness.borrow_mut() = false;
                    return ExitKind::Ok;
                }
            };

            update_coverage(6);
            if number % 2 == 0 {
                update_coverage(7);
                *is_divisible_by_2_harness.borrow_mut() = true;
                ExitKind::Ok
            } else {
                update_coverage(8);
                *is_divisible_by_2_harness.borrow_mut() = false;
                ExitKind::Ok
            }
        };

        let harness_executor = InProcessExecutor::new(
            &mut harness,
            tuple_list!(map_observer),
            &mut fuzzer,
            &mut state,
            &mut restarting_mgr,
        )
        .expect("Failed to create the Executor");

        let fandango_executor = FandangoParseExecutor::new(
            FandangoPythonModule::new(&args.fandango_file, &[]).unwrap(),
            is_divisible_by_2_observer_fandango.handle(),
            tuple_list!(
                is_divisible_by_2_observer_fandango,
                is_divisible_by_2_observer_harness
            ),
        );

        let mut executor = DiffExecutor::new(harness_executor, fandango_executor, tuple_list!());

        let mut stages = tuple_list!(StdMutationalStage::new(HavocScheduledMutator::new(
            havoc_mutations()
        )));

        // the fuzzer needs one initial input, otherwise the scheduler (obviously) isn't happy
        state
            .corpus_mut()
            .add(Testcase::new(BytesInput::new(b"42".to_vec())))?;

        fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut restarting_mgr)
    };

    match Launcher::builder()
        .shmem_provider(shmem_provider)
        .configuration(EventConfig::from_name("default"))
        .monitor(monitor)
        .run_client(&mut run_client)
        .cores(&args.cores)
        .broker_port(1337)
        // .stdout_file(Some("/dev/null"))
        .build()
        .launch()
    {
        Ok(()) => (),
        Err(Error::ShuttingDown) => println!("Fuzzing stopped by user. Good bye."),
        Err(err) => return Err(format!("Failed to run launcher: {err:?}")),
    }

    Ok(())
}

struct LogFeedback<'a> {
    is_divisible_by_2_observer_fandango: Handle<RefCellValueObserver<'a, u32>>,
    is_divisible_by_2_observer_harness: Handle<RefCellValueObserver<'a, bool>>,
}

impl<'a> LogFeedback<'a> {
    pub fn new(
        is_divisible_by_2_observer_fandango: Handle<RefCellValueObserver<'a, u32>>,
        is_divisible_by_2_observer_harness: Handle<RefCellValueObserver<'a, bool>>,
    ) -> Self {
        Self {
            is_divisible_by_2_observer_fandango,
            is_divisible_by_2_observer_harness,
        }
    }
}

impl<'a, EM, OT, S> Feedback<EM, BytesInput, OT, S> for LogFeedback<'a>
where
    OT: MatchNameRef,
{
    fn is_interesting(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _input: &BytesInput,
        _observers: &OT,
        _exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        Ok(false)
    }

    fn append_metadata(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        observers: &OT,
        testcase: &mut Testcase<BytesInput>,
    ) -> Result<(), Error> {
        let fan = *observers
            .get(&self.is_divisible_by_2_observer_fandango)
            .ok_or(Error::IllegalState(
                "is_divisible_by_2_observer_fandango not found".to_string(),
                ErrorBacktrace::new(),
            ))?
            .get_ref();
        let harness = *observers
            .get(&self.is_divisible_by_2_observer_harness)
            .ok_or(Error::IllegalState(
                "is_divisible_by_2_observer_harness not found".to_string(),
                ErrorBacktrace::new(),
            ))?
            .get_ref();
        testcase.add_metadata(LogMetadata { fan, harness });
        Ok(())
    }
}

#[derive(SerdeAny, Clone, Debug, Serialize, Deserialize)]
struct LogMetadata {
    fan: u32,
    harness: bool,
}

impl<'a> Named for LogFeedback<'a> {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("LogFeedback")
    }
}

impl<'a, S> StateInitializer<S> for LogFeedback<'a> {}
