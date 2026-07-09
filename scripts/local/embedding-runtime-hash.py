#!/usr/bin/env python3
"""Lightweight local hash-vector embedding runtime for resume-ir.

This command speaks the resume-ir-embedding-v1 stdout protocol with no third
party dependencies. It is a lexical hashing vectorizer for local performance
baselines and operator bring-up, not a semantic model quality claim.
"""

from __future__ import annotations

import hashlib
import math
import os
import sys
from dataclasses import dataclass
from typing import Iterable


BOUNDARY = "--resume-ir-embedding-input-boundary--"
INPUT_SCHEMA = "resume-ir-embedding-input-v1"
OUTPUT_SCHEMA = "resume-ir-embedding-v1"
PERSONALIZATION = b"resumeir"


@dataclass(frozen=True)
class EmbeddingRequest:
    model_id: str
    dimension: int
    inputs: list[tuple[str, str]]


def fail(message: str) -> None:
    print(f"embedding runtime blocked: {message}", file=sys.stderr)
    raise SystemExit(2)


def env_required(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        fail("required environment is missing")
    return value


def parse_input(path: str) -> EmbeddingRequest:
    try:
        with open(path, "r", encoding="utf-8") as handle:
            lines = handle.read().splitlines()
    except OSError:
        fail("input file is unavailable")

    if len(lines) < 4 or lines[0] != INPUT_SCHEMA:
        fail("input schema is unsupported")
    model_id = parse_header(lines[1], "model_id")
    dimension_text = parse_header(lines[2], "dimension")
    count_text = parse_header(lines[3], "count")
    try:
        dimension = int(dimension_text)
        count = int(count_text)
    except ValueError:
        fail("input header is invalid")
    if dimension <= 0 or count < 0:
        fail("input header is invalid")

    inputs: list[tuple[str, str]] = []
    index = 4
    while index < len(lines):
        input_header = lines[index]
        index += 1
        if not input_header.startswith("input=") or "\t" not in input_header:
            fail("input entry is invalid")
        identifier, _declared_len = input_header[len("input=") :].split("\t", 1)
        if not identifier or any(ch in identifier for ch in "\r\n\t"):
            fail("input identifier is invalid")
        if index >= len(lines) or lines[index] != "text:":
            fail("input text marker is missing")
        index += 1
        text_lines: list[str] = []
        while index < len(lines) and lines[index] != BOUNDARY:
            text_lines.append(lines[index])
            index += 1
        if index >= len(lines) or lines[index] != BOUNDARY:
            fail("input boundary is missing")
        index += 1
        inputs.append((identifier, "\n".join(text_lines)))

    if len(inputs) != count:
        fail("input count mismatch")
    return EmbeddingRequest(model_id=model_id, dimension=dimension, inputs=inputs)


def parse_header(line: str, key: str) -> str:
    prefix = f"{key}="
    if not line.startswith(prefix):
        fail("input header is invalid")
    value = line[len(prefix) :]
    if not value:
        fail("input header is invalid")
    return value


def tokenize(text: str) -> Iterable[str]:
    buffer: list[str] = []
    for char in text:
        if char.isascii() and (char.isalnum() or char in {"#", "+", "_"}):
            buffer.append(char.lower())
            continue
        if buffer:
            yield "".join(buffer)
            buffer.clear()
        if char.isspace():
            continue
        if char.isalnum():
            yield char.lower()
    if buffer:
        yield "".join(buffer)


def stable_digest(token: str) -> int:
    digest = hashlib.blake2b(
        token.encode("utf-8"),
        digest_size=8,
        person=PERSONALIZATION,
    ).digest()
    return int.from_bytes(digest, "little", signed=False)


def hash_vector(text: str, dimension: int) -> list[float]:
    values = [0.0] * dimension
    for token in tokenize(text):
        digest = stable_digest(token)
        index = digest % dimension
        sign = 1.0 if ((digest >> 63) & 1) == 0 else -1.0
        values[index] += sign

    magnitude = math.sqrt(sum(value * value for value in values))
    if magnitude > 0.0:
        values = [value / magnitude for value in values]
    return values


def format_float(value: float) -> str:
    text = f"{value:.9g}"
    return "0" if text == "-0" else text


def print_output(model_id: str, dimension: int, ids: list[str], vectors: list[list[float]]) -> None:
    print(OUTPUT_SCHEMA)
    print(f"model_id={model_id}")
    print(f"dimension={dimension}")
    for identifier, vector in zip(ids, vectors):
        if len(vector) != dimension or not all(math.isfinite(value) for value in vector):
            fail("hash vector is invalid")
        values = ",".join(format_float(value) for value in vector)
        print(f"vector={identifier}\t{values}")


def main() -> int:
    input_path = env_required("RESUME_IR_EMBEDDING_INPUT_PATH")
    expected_model_id = env_required("RESUME_IR_EMBEDDING_MODEL_ID")
    try:
        expected_dimension = int(env_required("RESUME_IR_EMBEDDING_DIMENSION"))
    except ValueError:
        fail("environment is invalid")
    if expected_dimension <= 0:
        fail("environment is invalid")

    request = parse_input(input_path)
    if request.model_id != expected_model_id or request.dimension != expected_dimension:
        fail("request does not match configured model")

    ids = [identifier for identifier, _text in request.inputs]
    vectors = [hash_vector(text, expected_dimension) for _identifier, text in request.inputs]
    print_output(expected_model_id, expected_dimension, ids, vectors)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
