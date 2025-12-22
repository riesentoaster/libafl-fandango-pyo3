use std::path::PathBuf;

use clap::Parser;
use libafl::{
    corpus::{Corpus, InMemoryCorpus, OnDiskCorpus, Testcase},
    events::{EventConfig, Launcher, LlmpRestartingEventManager, SendExiting as _},
    executors::{ExitKind, InProcessExecutor},
    feedbacks::CrashFeedback,
    fuzzer::{Fuzzer, StdFuzzer},
    inputs::{BytesInput, HasTargetBytes},
    monitors::MultiMonitor,
    mutators::{HavocScheduledMutator, havoc_mutations_no_crossover},
    schedulers::QueueScheduler,
    state::{HasCorpus, StdState},
};
use libafl_bolts::{
    Error,
    core_affinity::Cores,
    current_nanos,
    rands::StdRand,
    shmem::{ShMemProvider, StdShMemProvider},
    tuples::tuple_list,
};
use libafl_fandango_pyo3::{
    fandango::{FandangoPythonModule, FandangoPythonModuleInitError},
    libafl::FandangoPostMutationalStage,
};

#[derive(Parser)]
#[command(name = "run_fandango")]
#[command(about = "Run the fandango interface in Python")]
struct Args {
    #[arg(short, long, default_value = "examples/even_numbers.fan")]
    fandango_file: String,
    #[arg(short, long, value_parser = Cores::from_cmdline, default_value = "all")]
    cores: Cores,
    #[arg(short, long, default_value = "false")]
    print_inputs: bool,
    #[arg(short, long, default_value = "false")]
    quiet: bool,
    #[arg(short, long)]
    iters: Option<u64>,
    #[arg(short, long, default_value = "false")]
    violent_crash: bool,
}

pub fn main() -> Result<(), String> {
    env_logger::init();

    let args = Args::parse();
    let crash = || {
        if args.violent_crash {
            panic!("Violent crash");
        } else {
            ExitKind::Crash
        }
    };

    let monitor = MultiMonitor::new(|s| {
        if args.print_inputs || args.quiet {
            return;
        }
        println!("{s}")
    });

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

    let mut run_client = |state: Option<_>,
                          mut restarting_mgr: LlmpRestartingEventManager<_, _, _, _, _>,
                          _client_description| {
        log::info!("Running client");

        let mut objective = CrashFeedback::new();

        let mut state = state.unwrap_or_else(|| {
            StdState::new(
                StdRand::with_seed(current_nanos()),
                InMemoryCorpus::new(),
                OnDiskCorpus::new(PathBuf::from("./crashes")).unwrap(),
                &mut (),
                &mut objective,
            )
            .unwrap()
        });

        let mut fuzzer = StdFuzzer::new(QueueScheduler::new(), (), objective);

        let mut harness = |input: &BytesInput| {
            let target = input.target_bytes().to_vec();
            if args.print_inputs {
                hexdump::hexdump(&target);
            }
            let number = match String::from_utf8(target) {
                Ok(number) => number,
                Err(_) => return crash(),
            };

            let number = match number.parse::<u128>() {
                Ok(number) => number,
                Err(_) => return crash(),
            };

            if number % 2 == 0 {
                ExitKind::Ok
            } else {
                ExitKind::Crash
            }
        };

        let mut executor = InProcessExecutor::new(
            &mut harness,
            (),
            &mut fuzzer,
            &mut state,
            &mut restarting_mgr,
        )
        .expect("Failed to create the Executor");

        let fandango_module = FandangoPythonModule::new(&args.fandango_file, &[]).unwrap();

        // Number of times to clone and mutate each fandango-produced input (both inclusive)
        let min_iterations = 25;
        let max_iterations = 50;

        // Mutator to apply to each fandango-produced input — for simplicity's sake without crossover here, but you can also havoc crossover from your corpus
        let post_mutator =
            HavocScheduledMutator::with_max_stack_pow(havoc_mutations_no_crossover(), 3);

        let mut stages = tuple_list!(FandangoPostMutationalStage::new(
            fandango_module,
            post_mutator,
            min_iterations,
            max_iterations,
        ));

        // the fuzzer needs one initial input, otherwise the scheduler (obviously) isn't happy
        // in this example, this will never be used or mutated
        state
            .corpus_mut()
            .add(Testcase::new(BytesInput::new(b"42".to_vec())))?;

        if let Some(iters) = args.iters {
            for _ in 0..iters {
                fuzzer.fuzz_one(&mut stages, &mut executor, &mut state, &mut restarting_mgr)?;
            }
            restarting_mgr.on_shutdown()
        } else {
            fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut restarting_mgr)
        }
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
