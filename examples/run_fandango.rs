use clap::Parser;
use libafl_fandango_pyo3::fandango::{
    FandangoClient as _, FandangoInprocessModule, FandangoModuleInitError,
};

#[derive(Parser)]
#[command(name = "run_fandango")]
#[command(about = "Run the fandango interface in Python")]
struct Args {
    #[arg(short, long, default_value = "examples/even_numbers.fan")]
    fandango_file: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse();

    let mut fandango = match FandangoInprocessModule::new(&args.fandango_file, &[]) {
        Ok(fandango) => fandango,
        Err(FandangoModuleInitError::ModuleNotFoundError(e, tb)) => {
            return Err(format!(
                "A required Python module was not found. You may need to set the PYTHONPATH environment variable to the path of the Python interpreter, e.g. `export PYTHONPATH=$(echo .venv/lib/python*/site-packages)`. Underlying error:\n{}\n{}",
                e, tb
            ));
        }
        Err(e) => {
            return Err(format!("Error: {:?}", e));
        }
    };

    for _ in 0..10 {
        let input = fandango.next_input().unwrap();
        let num_parses = fandango.parse_input(&input).unwrap();
        println!(
            "{} can be parsed {} different ways", // this grammar always produces inputs that can be parsed in exactly one way — but this isn't necessarily true for all grammars
            String::from_utf8(input).unwrap(),
            num_parses
        );
        assert!(num_parses == 1);
    }

    Ok(())
}
