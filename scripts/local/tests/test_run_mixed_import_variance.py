#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "run-mixed-import-variance.py"


def load_runner():
    spec = importlib.util.spec_from_file_location("mixed_import_variance", SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ProtocolTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.runner = load_runner()

    def test_protocol_rejects_short_warmup_and_too_few_repeats(self) -> None:
        with self.assertRaises(self.runner.ProtocolInvalid):
            self.runner.validate_protocol(29, 5)
        with self.assertRaises(self.runner.ProtocolInvalid):
            self.runner.validate_protocol(30, 4)
        self.runner.validate_protocol(30, 5)

    def test_run_names_are_unique_and_formal_count_is_exact(self) -> None:
        names = self.runner.run_names(5)
        self.assertEqual(names, ["warmup", "formal-01", "formal-02", "formal-03", "formal-04", "formal-05"])
        self.assertEqual(len(names), len(set(names)))

    def test_time_parser_captures_resource_fields(self) -> None:
        parsed = self.runner.parse_time_output(
            """real 12.50
user 25.00
sys 4.00
 700000000 maximum resident set size
 10 page faults
 3 block input operations
 9 block output operations
 11 voluntary context switches
 13 involuntary context switches
 17 instructions retired
 19 cycles elapsed
"""
        )
        self.assertEqual(
            parsed,
            {
                "real_seconds": 12.5,
                "user_seconds": 25.0,
                "sys_seconds": 4.0,
                "peak_rss_bytes": 700000000,
                "page_faults": 10,
                "block_input_operations": 3,
                "block_output_operations": 9,
                "voluntary_context_switches": 11,
                "involuntary_context_switches": 13,
                "instructions_retired": 17,
                "cycles_elapsed": 19,
            },
        )

    def test_top_summary_discards_first_sample_and_derives_external_overlap(self) -> None:
        samples = [
            {"system_idle_percent": 0.0, "target_cpu_percent": 0.0, "system_disk_read_bytes": 0, "system_disk_written_bytes": 0},
            {"system_idle_percent": 70.0, "target_cpu_percent": 250.0, "system_disk_read_bytes": 100, "system_disk_written_bytes": 200},
            {"system_idle_percent": 65.0, "target_cpu_percent": 250.0, "system_disk_read_bytes": 300, "system_disk_written_bytes": 500},
            {"system_idle_percent": 90.0, "target_cpu_percent": 90.0, "system_disk_read_bytes": 700, "system_disk_written_bytes": 900},
        ]
        summary = self.runner.summarize_top_samples(samples, logical_cpus=10)
        self.assertEqual(summary["sample_count"], 3)
        self.assertEqual(summary["external_cpu_percent_samples"], [50.0, 100.0, 10.0])
        self.assertEqual(summary["sustained_external_overlap_fraction"], 0.0)
        self.assertEqual(summary["system_disk_read_bytes_delta"], 600)
        self.assertEqual(summary["system_disk_written_bytes_delta"], 700)

    def test_top_disk_byte_parser_is_binary_unit_aware(self) -> None:
        self.assertEqual(self.runner.parse_byte_count("1K"), 1024)
        self.assertEqual(self.runner.parse_byte_count("1.5G"), int(1.5 * 1024**3))

    def test_top_parser_correlates_host_disk_and_target_rows(self) -> None:
        parsed = self.runner.parse_top_output(
            """CPU usage: 10.0% user, 5.0% sys, 85.0% idle
Disks: 10/1G read, 20/2G written.
PID %CPU CSW TIME #TH STATE MEM
42 125.0 10 00:01 3 running 10M
CPU usage: 20.0% user, 5.0% sys, 75.0% idle
Disks: 12/1.5G read, 25/2.5G written.
PID %CPU CSW TIME #TH STATE MEM
42 200.0 20 00:02 3 running 10M
"""
        )
        self.assertEqual(len(parsed), 2)
        self.assertEqual(parsed[1]["target_cpu_percent"], 200.0)
        self.assertEqual(parsed[1]["system_disk_read_bytes"], int(1.5 * 1024**3))

    def test_shared_diagnostic_keeps_noisy_process_evidence_while_quiet_gate_rejects_it(self) -> None:
        base = {
            "telemetry_ok": True,
            "command_exit_code": 0,
            "semantic_ok": True,
            "thermal_states": ["nominal"],
            "memory_pressure_events": ["normal"],
            "pageouts_delta": 0,
            "swapouts_delta": 0,
        }
        short = dict(base, sustained_external_overlap_fraction=0.2)
        sustained = dict(base, sustained_external_overlap_fraction=0.6)
        self.assertEqual(
            self.runner.classify_validity(short),
            {
                "diagnostic": {"valid": True, "reasons": []},
                "quiet_acceptance": {"valid": True, "reasons": []},
            },
        )
        self.assertEqual(
            self.runner.classify_validity(sustained),
            {
                "diagnostic": {"valid": True, "reasons": []},
                "quiet_acceptance": {
                    "valid": False,
                    "reasons": ["sustained_external_cpu_overlap"],
                },
            },
        )

    def test_thermal_memory_vm_and_telemetry_fail_closed(self) -> None:
        observation = {
            "telemetry_ok": False,
            "command_exit_code": 2,
            "semantic_ok": False,
            "thermal_states": ["nominal", "serious"],
            "memory_pressure_events": ["normal", "warning"],
            "pageouts_delta": 1,
            "swapouts_delta": 1,
            "sustained_external_overlap_fraction": 0.0,
        }
        validity = self.runner.classify_validity(observation)
        self.assertFalse(validity["diagnostic"]["valid"])
        self.assertFalse(validity["quiet_acceptance"]["valid"])
        self.assertEqual(
            validity["diagnostic"]["reasons"],
            ["command_failed", "telemetry_failed", "semantic_drift"],
        )
        self.assertEqual(
            validity["quiet_acceptance"]["reasons"],
            [
                "command_failed",
                "telemetry_failed",
                "semantic_drift",
                "thermal_pressure",
                "memory_pressure",
                "pageout_growth",
                "swapout_growth",
            ],
        )

    def test_formal_summary_reports_all_runs_and_valid_subset(self) -> None:
        runs = []
        for index, full_ms in enumerate([100.0, 110.0, 120.0, 130.0, 140.0], 1):
            runs.append(
                {
                    "run_id": f"formal-{index:02d}",
                    "validity": {
                        "diagnostic": {"valid": True, "reasons": []},
                        "quiet_acceptance": {
                            "valid": index != 5,
                            "reasons": [] if index != 5 else ["thermal_pressure"],
                        },
                    },
                    "metrics": {
                        "full_import_ready_ms": full_ms,
                        "stage_parse_ms": full_ms - 10,
                        "stage_db_ms": full_ms - 20,
                        "stage_index_ms": full_ms - 30,
                        "peak_rss_bytes": 700_000_000 + index,
                    },
                }
            )
        summary = self.runner.summarize_formal_runs(runs, self.runner.QUIET_HOST_ACCEPTANCE)
        self.assertEqual(summary["formal_run_count"], 5)
        self.assertEqual(summary["diagnostic_valid_run_count"], 5)
        self.assertEqual(summary["quiet_acceptance_valid_run_count"], 4)
        self.assertEqual(summary["selected_valid_run_count"], 4)
        self.assertEqual(summary["median_valid_run_id"], "formal-02")
        self.assertEqual(summary["worst_valid_run_id"], "formal-04")
        self.assertIn("all_formal_runs", summary["variance"])
        self.assertIn("valid_formal_runs", summary["variance"])

    def test_formal_summary_does_not_crash_when_invalid_telemetry_omits_metrics(self) -> None:
        run = {
            "run_id": "formal-01",
            "validity": {
                "diagnostic": {"valid": False, "reasons": ["telemetry_failed"]},
                "quiet_acceptance": {"valid": False, "reasons": ["telemetry_failed"]},
            },
            "metrics": {"full_import_ready_ms": 100.0},
        }
        summary = self.runner.summarize_formal_runs(
            [run], self.runner.SHARED_HOST_DIAGNOSTIC
        )
        self.assertEqual(summary["selected_valid_run_count"], 0)
        self.assertIsNone(summary["variance"]["all_formal_runs"]["peak_rss_bytes"]["mean"])

    def test_public_summary_contains_no_private_fields(self) -> None:
        summary = self.runner.public_summary(
            experiment_id="opaque-experiment",
            mode=self.runner.SHARED_HOST_DIAGNOSTIC,
            runs=[],
            aggregate={
                "formal_run_count": 0,
                "diagnostic_valid_run_count": 0,
                "quiet_acceptance_valid_run_count": 0,
                "selected_valid_run_count": 0,
            },
            terminal="diagnostic_incomplete",
            claim="no_conclusion",
        )
        encoded = self.runner.canonical_json(summary)
        for forbidden_value in ["/Users/", "/home/", "PRIVATE-", "resume-cli import"]:
            self.assertNotIn(forbidden_value, encoded)
        self.assertEqual(summary["privacy"]["aggregate_only"], True)
        for key, value in summary["privacy"].items():
            if key != "aggregate_only":
                self.assertFalse(value)

    def test_shared_mode_can_never_emit_an_absolute_claim(self) -> None:
        aggregate = {
            "formal_run_count": 5,
            "diagnostic_valid_run_count": 5,
            "quiet_acceptance_valid_run_count": 0,
            "selected_valid_run_count": 5,
        }
        terminal, claim = self.runner.conclusion_for_mode(
            self.runner.SHARED_HOST_DIAGNOSTIC, aggregate
        )
        self.assertEqual((terminal, claim), ("diagnostic_observed", "diagnostic_only"))
        self.assertNotIn("absolute", claim)

    def test_quiet_mode_preserves_five_host_valid_run_gate(self) -> None:
        failed = {
            "formal_run_count": 5,
            "diagnostic_valid_run_count": 5,
            "quiet_acceptance_valid_run_count": 4,
            "selected_valid_run_count": 4,
        }
        passed = dict(failed, quiet_acceptance_valid_run_count=5, selected_valid_run_count=5)
        self.assertEqual(
            self.runner.conclusion_for_mode(self.runner.QUIET_HOST_ACCEPTANCE, failed),
            ("methodology_failed", "no_conclusion"),
        )
        self.assertEqual(
            self.runner.conclusion_for_mode(self.runner.QUIET_HOST_ACCEPTANCE, passed),
            ("quiet_host_accepted", "absolute_baseline_accepted"),
        )


if __name__ == "__main__":
    unittest.main()
