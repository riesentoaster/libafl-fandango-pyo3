# libafl-fandango (PyO3 edition)

This will allow you to run [Fandango](https://github.com/fandango-fuzzer/fandango) as a [LibAFL](https://github.com/aflplusplus/libafl) Generator, Mutator, or Executor.

It works by internally calling a python script using the [PyO3 interpreter](https://pyo3.rs). That script is expected to implement three functions. Here is the default implementation, but you can provide your own (using `FandangoPythonModule::with_custom_python_interface`):

```python
import os
from typing import Any
from fandango import Fandango



class FandangoWrapper:
    def __init__(self, fan_file: str, kwargs: dict[str, Any]):
        with open(fan_file) as f:
            self.fan = Fandango(f, **kwargs)
        self.generator = self.fan.generate_solutions()


def setup(fan_file: str, kwargs: dict[str, Any]) -> FandangoWrapper:
    return FandangoWrapper(fan_file, kwargs)


def next_input(wrapper: FandangoWrapper) -> bytes:
    return bytes(next(wrapper.generator))


def parse_input(wrapper: FandangoWrapper, input: bytes) -> int:
    return len(list(wrapper.fan.parse(input)))
```

## Examples

### Using the Fandango Rust Interface

Look at [the example](./examples/run_fandango.rs) for how to use the Rust interface to run Fandango. Run it using the following:

```bash
cargo run --example run_fandango --release -- --fandango-file  examples/even_numbers.fan
```

### Using it in a fuzzer

There are three ways of running libafl_fandango_pyo3 in LibAFL: As a generator, as a pseudo-mutator, and as an executor.

- The generator is the obvious and ideomatic answer.
- Using it as a pseudo-mutator is handy if you are building a mutational fuzzer anyway and just want to replace your mutator. Using it as a mutator will introduce a small performance benefit (running the scheduler, cloning the input to be mutated before it is immediately overwritten again, etc.), but compared to the overhead of running Python, I find this negligable. It also requires the corpus to not be empty (it needs to be primed) and a mutational stage to be created (make sure to only run one mutation to prevent unnecessary runtime).
- The executor can be used for differential fuzzing of any fuzzer built in LibAFL against a Fandango spec. Imagine you are testing a parser. You can write your harness in a way that writes to an observer if the input is deemed to be correct. Then you set up your fuzzer to use a parallel executor with Fandango's executor and compare the output of your harness with Fandango's opinion on whether the input is legal or not.

There are three example fuzzers: [baby_fuzzer_generator](./examples/baby_fuzzer_generator.rs), [baby_fuzzer_mutator](./examples/baby_fuzzer_mutator.rs), and [baby_fuzzer_differential](./examples/baby_fuzzer_differential.rs). The target for all three is an in-process function that parses the input to a string and then a number and checks if it is even. For the first two, it will consider any number that does not fit into 128 bits as a crash and thus produce a list of crashes after some time (in the crashes directory). They can be run with the following:

```bash
cargo run --example baby_fuzzer_generator --release
cargo run --example baby_fuzzer_mutator --release
cargo run --example baby_fuzzer_differential --release
```

## Known issues
For some reason, PyO3 struggles with matching the python interpreter to the one used in the shell – specifically when it comes to imports of dependencies. You may need to manually set the python path environment variable:

```bash
export PYTHONPATH=$(echo .venv/lib/python*/site-packages)
```
