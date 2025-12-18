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


if __name__ == "__main__":
    # path relative to this script
    fan_file = os.path.dirname(__file__) + "/even_numbers.fan"
    gen = setup(fan_file, {})
    for i in range(10):
        input = next_input(gen)
        print(input)
        assert parse_input(gen, input) > 0
    assert parse_input(gen, b"1") == 0, parse_input(gen, b"1")
    assert parse_input(gen, b"0") == 1, parse_input(gen, b"0")
    assert parse_input(gen, b"2") == 1, parse_input(gen, b"2")
