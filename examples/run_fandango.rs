use clap::Parser;
use libafl_fandango_pyo3::fandango::{FandangoPythonModule, FandangoPythonModuleInitError};

#[derive(Parser)]
#[command(name = "run_fandango")]
#[command(about = "Run the fandango interface in Python")]
struct Args {
    #[arg(short, long, default_value = "examples/even_numbers.fan")]
    fandango_file: String,
}

fn main() -> Result<(), String> {
    let args = Args::parse();

    let fandango = match FandangoPythonModule::new(&args.fandango_file, &[]) {
        Ok(fandango) => fandango,
        Err(FandangoPythonModuleInitError::PyErr(e)) => {
            return Err(format!(
                "You may need to set the PYTHONPATH environment variable to the path of the Python interpreter, e.g. `export PYTHONPATH=$(echo .venv/lib/python*/site-packages)`. Underlying error: {:?}",
                e
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
