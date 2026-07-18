#!/usr/bin/env python3
"""Local sentence-transformers adapter for resume-ir embedding runtimes.

This command speaks the resume-ir-embedding-v1 stdout protocol. It is intended
to run against a locally cached or locally installed sentence-transformers model
and defaults to offline/local-files-only loading.
"""

from __future__ import annotations

import math
import os
import sys
from contextlib import redirect_stderr, redirect_stdout
from dataclasses import dataclass
from typing import Callable, TypeVar
from typing import Iterable


BOUNDARY = "--resume-ir-embedding-input-boundary--"
INPUT_SCHEMA = "resume-ir-embedding-input-v1"
OUTPUT_SCHEMA = "resume-ir-embedding-v1"
T = TypeVar("T")


@dataclass(frozen=True)
class EmbeddingRequest:
    model_id: str
    dimension: int
    inputs: list[tuple[str, str]]


def fail(message: str) -> None:
    print(f"embedding runtime blocked: {message}", file=sys.stderr)
    raise SystemExit(2)


def run_third_party_quietly(callback: Callable[[], T]) -> T:
    with open(os.devnull, "w", encoding="utf-8") as sink:
        with redirect_stdout(sink), redirect_stderr(sink):
            return callback()


def env_required(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        fail("required environment is missing")
    return value


def parse_bool_env(name: str, default: bool) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    return value.strip().lower() not in {"0", "false", "no", "off"}


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


def load_sentence_transformer(model_name: str, cache_folder: str | None, local_files_only: bool):
    if local_files_only:
        os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
        os.environ.setdefault("HF_HUB_OFFLINE", "1")

    def import_sentence_transformer():
        from sentence_transformers import SentenceTransformer

        return SentenceTransformer

    try:
        SentenceTransformer = run_third_party_quietly(import_sentence_transformer)
    except Exception:
        fail("sentence-transformers is not installed")

    try:
        return run_third_party_quietly(
            lambda: SentenceTransformer(
                model_name,
                cache_folder=cache_folder,
                local_files_only=local_files_only,
            )
        )
    except TypeError:
        if not local_files_only:
            return run_third_party_quietly(
                lambda: SentenceTransformer(model_name, cache_folder=cache_folder)
            )
        fail("installed sentence-transformers does not support local-only loading")
    except Exception:
        fail("local sentence-transformers model is unavailable")


def encode(model, texts: list[str]) -> list[list[float]]:
    try:
        vectors = run_third_party_quietly(
            lambda: model.encode(
                texts,
                normalize_embeddings=True,
                show_progress_bar=False,
            )
        )
    except Exception:
        fail("model encoding failed")
    return [coerce_vector(vector) for vector in vectors]


def coerce_vector(vector: Iterable[float]) -> list[float]:
    values = [float(value) for value in vector]
    if not all(math.isfinite(value) for value in values):
        fail("model returned invalid vector")
    return values


def format_float(value: float) -> str:
    text = f"{value:.9g}"
    return "0" if text == "-0" else text


def print_output(model_id: str, dimension: int, ids: list[str], vectors: list[list[float]]) -> None:
    if len(ids) != len(vectors):
        fail("model returned wrong vector count")
    print(OUTPUT_SCHEMA)
    print(f"model_id={model_id}")
    print(f"dimension={dimension}")
    for identifier, vector in zip(ids, vectors):
        if len(vector) != dimension:
            fail("model returned wrong vector dimension")
        values = ",".join(format_float(value) for value in vector)
        print(f"vector={identifier}\t{values}")


def main() -> int:
    input_path = env_required("RESUME_IR_EMBEDDING_INPUT_PATH")
    expected_model_id = env_required("RESUME_IR_EMBEDDING_MODEL_ID")
    expected_dimension = int(env_required("RESUME_IR_EMBEDDING_DIMENSION"))
    request = parse_input(input_path)
    if request.model_id != expected_model_id or request.dimension != expected_dimension:
        fail("request does not match configured model")

    model_name = os.environ.get("RESUME_IR_SENTENCE_TRANSFORMERS_MODEL", expected_model_id)
    cache_folder = os.environ.get("RESUME_IR_SENTENCE_TRANSFORMERS_CACHE")
    local_files_only = not parse_bool_env("RESUME_IR_SENTENCE_TRANSFORMERS_ALLOW_DOWNLOAD", False)
    model = load_sentence_transformer(model_name, cache_folder, local_files_only)
    ids = [identifier for identifier, _text in request.inputs]
    texts = [text for _identifier, text in request.inputs]
    vectors = encode(model, texts)
    print_output(expected_model_id, expected_dimension, ids, vectors)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError:
        fail("environment is invalid")
