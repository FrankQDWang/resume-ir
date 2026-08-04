#!/usr/bin/env python3
"""Convert the frozen multilingual E5 source model into a local Core ML screen."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import coremltools as ct
import numpy as np
import torch
import torch.nn.functional as functional
from transformers import AutoModel, AutoTokenizer


MODEL_ID = "intfloat/multilingual-e5-small"
REVISION = "614241f622f53c4eeff9890bdc4f31cfecc418b3"
BATCH, TOKENS, DIMENSION = 4, 512, 384
SYNTHETIC_TEXTS = (
    "passage: " + "distributed systems reliability engineering observability " * 110,
    "passage: " + "machine learning retrieval ranking evaluation embeddings " * 110,
    "passage: " + "financial analysis forecasting risk controls compliance " * 110,
    "passage: " + "product design accessibility research prototyping systems " * 110,
)


class E5Embedding(torch.nn.Module):
    def __init__(self, model: torch.nn.Module) -> None:
        super().__init__()
        self.model = model
        self.register_buffer(
            "position_ids",
            torch.arange(TOKENS, dtype=torch.int64).unsqueeze(0).expand(BATCH, -1),
        )
        self.register_buffer(
            "token_type_ids", torch.zeros((BATCH, TOKENS), dtype=torch.int64)
        )

    def forward(
        self, input_ids: torch.Tensor, attention_mask: torch.Tensor
    ) -> torch.Tensor:
        hidden = self.model(
            input_ids=input_ids.to(torch.int64),
            attention_mask=attention_mask.to(torch.int64),
            position_ids=self.position_ids,
            token_type_ids=self.token_type_ids,
            return_dict=False,
        )[0]
        mask = attention_mask.unsqueeze(-1).to(hidden.dtype)
        pooled = (hidden * mask).sum(dim=1) / mask.sum(dim=1).clamp(min=1.0)
        return functional.normalize(pooled, p=2, dim=1)


def write_tensor(path: Path, values: np.ndarray, dtype: np.dtype[object]) -> None:
    contiguous = np.ascontiguousarray(values, dtype=dtype)
    path.write_bytes(contiguous.tobytes(order="C"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=False)

    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, revision=REVISION)
    source = AutoModel.from_pretrained(MODEL_ID, revision=REVISION).eval()
    wrapped = E5Embedding(source).eval()
    encoded = tokenizer(
        SYNTHETIC_TEXTS,
        padding="max_length",
        truncation=True,
        max_length=TOKENS,
        return_tensors="pt",
    )
    input_ids = encoded["input_ids"].to(torch.int32)
    attention_mask = encoded["attention_mask"].to(torch.int32)
    if tuple(input_ids.shape) != (BATCH, TOKENS):
        raise RuntimeError("tokenizer did not produce the frozen B4x512 shape")

    with torch.inference_mode():
        reference = wrapped(input_ids, attention_mask)
        traced = torch.jit.trace(wrapped, (input_ids, attention_mask), strict=True)
        replay = traced(input_ids, attention_mask)
    if tuple(reference.shape) != (BATCH, DIMENSION) or not torch.allclose(
        reference, replay, rtol=1e-5, atol=1e-5
    ):
        raise RuntimeError("traced source model did not preserve the reference output")

    package = ct.convert(
        traced,
        convert_to="mlprogram",
        minimum_deployment_target=ct.target.macOS15,
        compute_precision=ct.precision.FLOAT16,
        inputs=[
            ct.TensorType(name="input_ids", shape=(BATCH, TOKENS), dtype=np.int32),
            ct.TensorType(
                name="attention_mask", shape=(BATCH, TOKENS), dtype=np.int32
            ),
        ],
        outputs=[ct.TensorType(name="embeddings", dtype=np.float32)],
    )
    package.author = "resume-ir local Issue #380 experiment"
    package.short_description = "Fixed B4x512 multilingual E5 FP16 tensor screen"
    package.save(output_dir / "e5-b4x512.mlpackage")

    write_tensor(output_dir / "input_ids.i32le", input_ids.numpy(), np.dtype("<i4"))
    write_tensor(
        output_dir / "attention_mask.i32le",
        attention_mask.numpy(),
        np.dtype("<i4"),
    )
    write_tensor(
        output_dir / "pytorch_reference.f32le",
        reference.numpy(),
        np.dtype("<f4"),
    )
    (output_dir / "synthetic_texts.json").write_text(
        json.dumps(SYNTHETIC_TEXTS, separators=(",", ":")), encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "schema_version": "resume-ir.coreml-b4x512-conversion.v1",
                "model_revision_matches": True,
                "batch": BATCH,
                "tokens": TOKENS,
                "dimension": DIMENSION,
                "source_trace_matches": True,
                "contains_private_data": False,
            },
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
