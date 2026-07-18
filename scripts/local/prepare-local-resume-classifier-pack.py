#!/usr/bin/env python3
"""Convert one reviewed local sklearn classifier into the desktop runtime pack.

The source and generated model remain local build inputs. Only the aggregate
runtime-pack manifest is intended to be checked into the repository.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import stat
import tempfile
from typing import Any

import joblib


PACK_SCHEMA = "resume-ir.desktop-classifier-model-pack.v1"
ARTIFACT_SCHEMA = "resume_ir_linear_promotion_v1"
CLASSIFIER_EPOCH = "precision_first_v4"
FEATURE_CONTRACT = "bounded_normalized_text_plus_structure_v1"
FAMILY = "tfidf_logistic_regression"
MODEL_FILE = "linear-promotion-model.json"
MAX_FEATURES = 250_000
MAX_INPUT_CHARS = 32_768
MAX_MODEL_BYTES = 32 * 1024 * 1024


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Prepare a local, user-authorized desktop classifier resource pack."
    )
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args()


def require_direct_private_file(path: Path) -> Path:
    if not path.is_absolute():
        raise ValueError("source must be absolute")
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        raise ValueError("source must be a regular non-symlink file")
    if metadata.st_size <= 0 or metadata.st_mode & 0o077:
        raise ValueError("source must be non-empty and owner-only")
    return path


def exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError(f"{label} contract is invalid")
    return value


def finite_float(value: Any, label: str) -> float:
    converted = float(value)
    if not math.isfinite(converted):
        raise ValueError(f"{label} must be finite")
    return converted


def convert(source: Path) -> tuple[bytes, dict[str, Any]]:
    trained = exact_keys(
        joblib.load(source),
        {
            "classifier",
            "config",
            "family",
            "feature_contract",
            "input_cap",
            "threshold",
            "vectorizer",
        },
        "trained classifier",
    )
    if trained["family"] != FAMILY or trained["feature_contract"] != FEATURE_CONTRACT:
        raise ValueError("trained classifier identity is invalid")
    input_cap = int(trained["input_cap"])
    threshold = finite_float(trained["threshold"], "threshold")
    if input_cap != MAX_INPUT_CHARS or not 0.0 < threshold <= 1.0:
        raise ValueError("trained classifier bounds are invalid")

    vectorizer = trained["vectorizer"]
    classifier = trained["classifier"]
    if (
        vectorizer.analyzer != "char"
        or tuple(vectorizer.ngram_range) != (3, 5)
        or vectorizer.lowercase is not True
        or vectorizer.sublinear_tf is not True
        or vectorizer.norm != "l2"
    ):
        raise ValueError("vectorizer contract is invalid")
    vocabulary = vectorizer.vocabulary_
    if not isinstance(vocabulary, dict) or not 0 < len(vocabulary) <= MAX_FEATURES:
        raise ValueError("vectorizer vocabulary is invalid")
    if set(vocabulary.values()) != set(range(len(vocabulary))):
        raise ValueError("vectorizer vocabulary indices are invalid")
    if tuple(classifier.classes_.tolist()) != (0, 1):
        raise ValueError("classifier classes are invalid")
    if classifier.coef_.shape != (1, len(vocabulary)) or classifier.intercept_.shape != (1,):
        raise ValueError("classifier coefficient shape is invalid")
    if len(vectorizer.idf_) != len(vocabulary):
        raise ValueError("vectorizer idf shape is invalid")

    ordered = sorted(vocabulary.items(), key=lambda item: item[1])
    features = []
    for ngram, index in ordered:
        if not isinstance(ngram, str) or not 3 <= len(ngram) <= 5:
            raise ValueError("vectorizer ngram is invalid")
        idf = finite_float(vectorizer.idf_[index], "idf")
        coefficient = finite_float(classifier.coef_[0, index], "coefficient")
        if idf <= 0.0:
            raise ValueError("idf must be positive")
        features.append({"ngram": ngram, "idf": idf, "coefficient": coefficient})

    artifact = {
        "schema": ARTIFACT_SCHEMA,
        "classifier_epoch": CLASSIFIER_EPOCH,
        "feature_contract": FEATURE_CONTRACT,
        "max_input_chars": input_cap,
        "threshold": threshold,
        "intercept": finite_float(classifier.intercept_[0], "intercept"),
        "features": features,
    }
    model_json = json.dumps(
        artifact,
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
    )
    model_sha256 = hashlib.sha256(model_json.encode()).hexdigest()
    envelope = json.dumps(
        {"model_json": model_json, "model_sha256": model_sha256},
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
    ).encode()
    if not 0 < len(envelope) <= MAX_MODEL_BYTES:
        raise ValueError("converted classifier exceeds the runtime bound")
    manifest = {
        "schema_version": PACK_SCHEMA,
        "classifier_epoch": CLASSIFIER_EPOCH,
        "feature_contract": FEATURE_CONTRACT,
        "distribution_scope": "user_authorized_internal_test",
        "network_access": "disabled",
        "files": [
            {
                "role": "linear_promotion_model",
                "file": MODEL_FILE,
                "bytes": len(envelope),
                "sha256": hashlib.sha256(envelope).hexdigest(),
            }
        ],
    }
    return envelope, manifest


def atomic_write(path: Path, body: bytes, mode: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(body)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main() -> int:
    args = parse_args()
    source = require_direct_private_file(args.source)
    output_dir = args.output_dir
    if not output_dir.is_absolute() or output_dir.is_symlink():
        raise ValueError("output directory must be an absolute non-symlink path")
    envelope, manifest = convert(source)
    atomic_write(output_dir / MODEL_FILE, envelope, 0o600)
    manifest_body = (json.dumps(manifest, indent=2, ensure_ascii=False) + "\n").encode()
    atomic_write(output_dir / "runtime-pack.json", manifest_body, 0o600)
    print(
        json.dumps(
            {
                "schema_version": "resume-ir.local-classifier-pack-preparation.v1",
                "feature_count": len(json.loads(json.loads(envelope)["model_json"])["features"]),
                "model_bytes": len(envelope),
                "status": "prepared",
            },
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
