from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "coreml-product-path-experiment.py"
SPEC = importlib.util.spec_from_file_location("product_experiment", SCRIPT)
assert SPEC and SPEC.loader
module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = module
SPEC.loader.exec_module(module)


class ProductExperimentTests(unittest.TestCase):
    def test_product_runner_accepts_the_production_coreml_tokenizer_identity(self) -> None:
        manifest = json.loads(
            (
                module.REPO
                / "apps/desktop/resources/embedding/aarch64-apple-darwin/coreml-tokenizer-pack.json"
            ).read_text(encoding="utf-8")
        )
        self.assertEqual(manifest["schema_version"], module.COREML_TOKENIZER_SCHEMA)
        self.assertEqual(manifest["model_id"], module.MODEL_ID)

    def test_coreml_manifest_pin_matches_authoritative_pack(self) -> None:
        manifest = (module.REPO / "apps/desktop/resources/embedding/aarch64-apple-darwin/coreml-runtime-pack.json").read_bytes()
        source = (module.REPO / "crates/embedding-runtime/src/coreml_product_experiment.rs").read_text()
        self.assertIn(f'const MANIFEST_SHA256: &str = "{hashlib.sha256(manifest).hexdigest()}";', source)
        self.assertIn(f'validate_file(&root.join("runtime-pack.json"), {len(manifest):_}, MANIFEST_SHA256)?;', source)

    def test_pack_verification_checks_identity_size_and_digest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = b"reviewed-runtime"
            (root / "payload.bin").write_bytes(payload)
            (root / "runtime-pack.json").write_text(
                json.dumps(
                    {
                        "schema_version": "example.v1",
                        "model_id": "expected",
                        "files": [
                            {
                                "role": "model",
                                "file": "payload.bin",
                                "bytes": len(payload),
                                "sha256": hashlib.sha256(payload).hexdigest(),
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            verified = module.verify_pack(root, "example.v1", {"model_id": "expected"})
            self.assertEqual(verified["model_id"], "expected")
            (root / "payload.bin").write_bytes(b"tampered-runtime")
            with self.assertRaisesRegex(module.ExperimentError, "runtime_pack_digest_mismatch"):
                module.verify_pack(root, "example.v1", {"model_id": "expected"})

    def test_staging_removes_platform_suffix_from_all_runtime_names(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, destination = root / "source", root / "destination"
            source.mkdir()
            for final_name in module.RUNTIME_NAMES.values():
                binary = source / f"{final_name}-{module.TARGET}"
                binary.write_bytes(b"executable")
                binary.chmod(0o700)
            staged = module.stage_executables(destination, source)
            self.assertEqual(
                {path.name for path in staged.values()}, set(module.RUNTIME_NAMES.values())
            )

    def test_scan_submission_is_attempted_exactly_once_when_outcome_is_unknown(self) -> None:
        class UnknownTransport:
            def __init__(self) -> None:
                self.paths: list[str] = []

            def request(self, _method: str, path: str, _payload: object):
                self.paths.append(path)
                if path == "/source-roots/register":
                    return 200, {"root": {"root_id": "root-1"}}
                raise module.ExperimentError("ipc_request_failed")

        transport = UnknownTransport()
        with self.assertRaisesRegex(module.ExperimentError, "scan_submission_unknown"):
            module.register_and_scan(transport, Path("/private-redacted"), lambda: 4.0)
        self.assertEqual(transport.paths, ["/source-roots/register", "/source-roots/scan"])

    def test_terminal_fence_binds_root_scan_counts_and_non_ocr_queues(self) -> None:
        counts = {
            "discovered": 8,
            "searchable": 6,
            "ocr": 1,
            "failed": 1,
            "ignored": 0,
            "processed": 8,
        }
        progress = {
            "latest_import_scan": {
                "files_discovered": 8,
                "searchable_documents": 6,
                "ocr_required_documents": 1,
                "failed_documents": 1,
                "ignored_entries": 0,
            }
        }
        roots = {
            "roots": [
                {
                    "root_id": "root-1",
                    "last_scan": {
                        "scan_id": "scan-1",
                        "phase": "complete",
                        "completeness": "complete",
                        "counts": counts,
                    },
                }
            ]
        }
        status = {
            "import_tasks_queued": 0,
            "import_tasks_recoverable": 0,
            "recovery_queue_depth": 0,
            "embedding_queue_depth": 0,
            "ocr_queue_depth": 1,
            "index_health": "ready",
            "indexed_documents": 6,
        }
        self.assertEqual(
            module.terminal_counts("root-1", "scan-1", progress, roots, status),
            progress["latest_import_scan"],
        )
        status["embedding_queue_depth"] = 1
        self.assertIsNone(
            module.terminal_counts("root-1", "scan-1", progress, roots, status)
        )

    def test_run_count_is_bounded(self) -> None:
        self.assertEqual(module.bounded_runs("2"), 2)
        for invalid in ("0", "11", "not-a-number"):
            with self.assertRaises(Exception):
                module.bounded_runs(invalid)


if __name__ == "__main__":
    unittest.main()
