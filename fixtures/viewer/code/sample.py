#!/usr/bin/env python3
"""A small script for the viewer's tests."""

import sys

LIMIT = 10


class Counter:
    """Counts what it is given."""

    def __init__(self, start=0):
        self.count = start

    def add(self, amount):
        # Never past the limit.
        self.count = min(self.count + amount, LIMIT)
        return self.count


def main(args):
    counter = Counter()
    for arg in args:
        counter.add(int(arg))
    print(f"Counted {counter.count}")


if __name__ == "__main__":
    main(sys.argv[1:])
