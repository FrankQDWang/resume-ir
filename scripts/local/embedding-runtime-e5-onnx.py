#!/usr/bin/env python3
"""Local multilingual E5 ONNX adapter for resume-ir embedding workers.

This command speaks the resume-ir-embedding-v1 stdout protocol for
intfloat/multilingual-e5-small. It loads tokenizer and ONNX files from a local
directory only, prefixes retrieval inputs as required by E5, and never prints
raw input text or local model paths.
"""

from __future__ import annotations

import math
import os
import sys
from contextlib import redirect_stderr, redirect_stdout
from dataclasses import dataclass
from typing import Any, Callable, Iterable, TypeVar


BOUNDARY = "--resume-ir-embedding-input-boundary--"
INPUT_SCHEMA = "resume-ir-embedding-input-v1"
OUTPUT_SCHEMA = "resume-ir-embedding-v1"
MODEL_ID = "intfloat/multilingual-e5-small"
DIMENSION = 384
DEFAULT_MAX_LENGTH = 512
DEFAULT_ONNX_FILE = "onnx/model.onnx"
FALLBACK_ONNX_FILE = "model.onnx"
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


def parse_positive_int(text: str) -> int:
    try:
        value = int(text)
    except ValueError:
        fail("environment is invalid")
    if value <= 0:
        fail("environment is invalid")
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


def resolve_onnx_path(model_dir: str) -> str:
    selector = os.environ.get("RESUME_IR_E5_ONNX_FILE", "").strip()
    if selector:
        return require_existing_local_file(model_dir, selector)
    default_path = safe_join(model_dir, DEFAULT_ONNX_FILE)
    if os.path.isfile(default_path):
        return default_path
    return require_existing_local_file(model_dir, FALLBACK_ONNX_FILE)


def require_existing_local_file(model_dir: str, selector: str) -> str:
    candidate = safe_join(model_dir, selector)
    if not os.path.isfile(candidate):
        fail("local E5 ONNX model file is unavailable")
    return candidate


def safe_join(root: str, relative_path: str) -> str:
    normalized = os.path.normpath(relative_path)
    if (
        not normalized
        or normalized == "."
        or normalized == ".."
        or os.path.isabs(relative_path)
        or normalized.startswith(f"..{os.sep}")
    ):
        fail("local E5 ONNX file selector is invalid")
    return os.path.join(root, normalized)


def load_modules():
    try:
        return run_third_party_quietly(import_modules)
    except Exception:
        fail("local E5 ONNX dependencies are unavailable")


def import_modules():
    import numpy as np
    import onnxruntime as ort
    from transformers import AutoTokenizer

    return np, ort, AutoTokenizer


def load_tokenizer(AutoTokenizer: Any, model_dir: str):
    try:
        return run_third_party_quietly(
            lambda: AutoTokenizer.from_pretrained(
                model_dir,
                local_files_only=True,
                use_fast=True,
            )
        )
    except Exception:
        fail("local E5 tokenizer is unavailable")


def load_session(ort: Any, onnx_path: str):
    try:
        options = ort.SessionOptions()
        return run_third_party_quietly(
            lambda: ort.InferenceSession(
                onnx_path,
                sess_options=options,
                providers=["CPUExecutionProvider"],
            )
        )
    except Exception:
        fail("local E5 ONNX session is unavailable")


def e5_input(identifier: str, text: str) -> str:
    if identifier == "query":
        return f"query: {text}"
    return f"passage: {text}"


def encode(np: Any, tokenizer: Any, session: Any, inputs: list[tuple[str, str]], max_length: int) -> list[list[float]]:
    prefixed_texts = [e5_input(identifier, text) for identifier, text in inputs]
    try:
        encoded = run_third_party_quietly(
            lambda: tokenizer(
                prefixed_texts,
                max_length=max_length,
                padding=True,
                truncation=True,
                return_tensors="np",
            )
        )
    except Exception:
        fail("local E5 tokenization failed")
    encoded = {key: np.asarray(value) for key, value in encoded.items()}
    if "input_ids" not in encoded or "attention_mask" not in encoded:
        fail("local E5 tokenizer output is invalid")

    feed = build_onnx_feed(np, session, encoded)
    try:
        outputs = run_third_party_quietly(lambda: session.run(None, feed))
    except Exception:
        fail("local E5 ONNX inference failed")
    return pooled_vectors(np, outputs, encoded["attention_mask"])


def build_onnx_feed(np: Any, session: Any, encoded: dict[str, Any]) -> dict[str, Any]:
    feed: dict[str, Any] = {}
    try:
        input_names = [item.name for item in session.get_inputs()]
    except Exception:
        fail("local E5 ONNX session inputs are invalid")
    for name in input_names:
        if name in encoded:
            feed[name] = encoded[name]
        elif name == "token_type_ids":
            feed[name] = np.zeros_like(encoded["input_ids"])
        else:
            fail("local E5 tokenizer output does not match ONNX inputs")
    return feed


def pooled_vectors(np: Any, outputs: Any, attention_mask: Any) -> list[list[float]]:
    if not outputs:
        fail("local E5 ONNX output is invalid")
    values = np.asarray(outputs[0], dtype=np.float32)
    if values.ndim == 3:
        vectors = average_pool(np, values, attention_mask)
    elif values.ndim == 2:
        vectors = values
    else:
        fail("local E5 ONNX output is invalid")
    vectors = l2_normalize(np, vectors)
    return [[float(value) for value in vector] for vector in vectors]


def average_pool(np: Any, last_hidden_state: Any, attention_mask: Any):
    mask = np.asarray(attention_mask, dtype=np.float32)
    if mask.ndim != 2 or last_hidden_state.ndim != 3:
        fail("local E5 pooling input is invalid")
    expanded_mask = np.expand_dims(mask, axis=-1)
    summed = (last_hidden_state * expanded_mask).sum(axis=1)
    counts = np.maximum(mask.sum(axis=1, keepdims=True), 1e-12)
    return summed / counts


def l2_normalize(np: Any, vectors: Any):
    norms = np.linalg.norm(vectors, axis=1, keepdims=True)
    if bool(np.any(norms <= 0.0)):
        fail("model returned invalid vector")
    return vectors / norms


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
        values = coerce_vector(vector)
        if len(values) != dimension:
            fail("model returned wrong vector dimension")
        text = ",".join(format_float(value) for value in values)
        print(f"vector={identifier}\t{text}")


def main() -> int:
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
    os.environ.setdefault("HF_HUB_OFFLINE", "1")

    input_path = env_required("RESUME_IR_EMBEDDING_INPUT_PATH")
    expected_model_id = env_required("RESUME_IR_EMBEDDING_MODEL_ID")
    expected_dimension = parse_positive_int(env_required("RESUME_IR_EMBEDDING_DIMENSION"))
    if expected_model_id != MODEL_ID or expected_dimension != DIMENSION:
        fail("configured E5 model is unsupported")

    request = parse_input(input_path)
    if request.model_id != expected_model_id or request.dimension != expected_dimension:
        fail("request does not match configured model")

    ids = [identifier for identifier, _text in request.inputs]
    if not request.inputs:
        print_output(expected_model_id, expected_dimension, ids, [])
        return 0

    model_dir = env_required("RESUME_IR_E5_MODEL_DIR")
    max_length = parse_positive_int(os.environ.get("RESUME_IR_E5_MAX_LENGTH", str(DEFAULT_MAX_LENGTH)))
    np, ort, AutoTokenizer = load_modules()
    tokenizer = load_tokenizer(AutoTokenizer, model_dir)
    session = load_session(ort, resolve_onnx_path(model_dir))
    vectors = encode(np, tokenizer, session, request.inputs, max_length)
    print_output(expected_model_id, expected_dimension, ids, vectors)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
