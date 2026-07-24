#!/usr/bin/env python3
"""Mutation regressions for the S809 delivery and v29 correctness contracts."""

from __future__ import annotations

import copy
import importlib.util
import pathlib
import tomllib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


def load_module(path: pathlib.Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"unable to load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def tampered(value: object) -> object:
    if isinstance(value, bool):
        return not value
    if isinstance(value, int):
        return value + 1
    if isinstance(value, str):
        return f"{value}__tampered"
    if isinstance(value, list):
        return [*value, "__tampered"]
    raise AssertionError(f"no mutation strategy for {value!r}")


class GovernanceContractMutationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.autonomous_checker = load_module(
            ROOT / "scripts" / "ci" / "check-autonomous-goal.py",
            "autonomous_goal_checker_under_test",
        )
        cls.performance_checker = load_module(
            ROOT / "scripts" / "ci" / "check-performance-contracts.py",
            "performance_contract_checker_under_test",
        )
        with (ROOT / "ACTIVE_GOAL.toml").open("rb") as fh:
            cls.active_goal = tomllib.load(fh)
        with (ROOT / "perf" / "acceptance-matrix.toml").open("rb") as fh:
            cls.matrix = tomllib.load(fh)

    def validate_delivery(self, active_goal: dict) -> None:
        autonomous = active_goal["autonomous_delivery"]
        active_slice = active_goal["scope"]["active_slice"]
        self.autonomous_checker.validate_correctness_delivery_sequence(
            autonomous,
            active_slice,
            autonomous["transitions"],
        )

    def test_live_contracts_pass_focused_validators(self) -> None:
        self.validate_delivery(copy.deepcopy(self.active_goal))
        self.performance_checker.validate_forward_migration_feature_train(
            copy.deepcopy(self.matrix)
        )

    def test_each_delivery_transition_is_required(self) -> None:
        expected = self.autonomous_checker.CORRECTNESS_DELIVERY_TRANSITIONS
        for name in expected:
            with self.subTest(name=name):
                mutated_goal = copy.deepcopy(self.active_goal)
                transitions = mutated_goal["autonomous_delivery"]["transitions"]
                mutated_goal["autonomous_delivery"]["transitions"] = [
                    transition
                    for transition in transitions
                    if transition["name"] != name
                ]
                with self.assertRaises(ValueError):
                    self.validate_delivery(mutated_goal)

    def test_each_delivery_transition_field_is_exact(self) -> None:
        expected = self.autonomous_checker.CORRECTNESS_DELIVERY_TRANSITIONS
        for name, shape in expected.items():
            for field in [
                "from",
                "to",
                "required_permissions",
                "required_evidence",
                "allowed_actions",
            ]:
                with self.subTest(name=name, field=field):
                    mutated_goal = copy.deepcopy(self.active_goal)
                    transition = next(
                        item
                        for item in mutated_goal["autonomous_delivery"]["transitions"]
                        if item["name"] == name
                    )
                    transition[field] = tampered(shape[field])
                    with self.assertRaises(ValueError):
                        self.validate_delivery(mutated_goal)

            with self.subTest(name=name, field="name"):
                mutated_goal = copy.deepcopy(self.active_goal)
                transition = next(
                    item
                    for item in mutated_goal["autonomous_delivery"]["transitions"]
                    if item["name"] == name
                )
                transition["name"] = f"{name}__tampered"
                with self.assertRaises(ValueError):
                    self.validate_delivery(mutated_goal)

            with self.subTest(name=name, field="unexpected"):
                mutated_goal = copy.deepcopy(self.active_goal)
                transition = next(
                    item
                    for item in mutated_goal["autonomous_delivery"]["transitions"]
                    if item["name"] == name
                )
                transition["bypass"] = True
                with self.assertRaises(ValueError):
                    self.validate_delivery(mutated_goal)

    def test_delivery_policy_fields_are_required_and_exact(self) -> None:
        cases = [
            (("scope", "active_slice", "scope_exception"), True),
            (
                ("autonomous_delivery", "permissions", "protected_merge_allowed"),
                True,
            ),
            (
                ("autonomous_delivery", "permissions", "branch_cleanup_allowed"),
                True,
            ),
            (
                ("autonomous_delivery", "permissions", "local_install_allowed"),
                True,
            ),
            (
                (
                    "autonomous_delivery",
                    "permissions",
                    "private_resume_root_read_allowed",
                ),
                True,
            ),
            (
                ("autonomous_delivery", "permissions", "direct_main_push_allowed"),
                False,
            ),
            (
                ("autonomous_delivery", "permissions", "admin_bypass_allowed"),
                False,
            ),
            (
                (
                    "autonomous_delivery",
                    "pr_budget",
                    "allow_scope_exception_auto_merge",
                ),
                False,
            ),
            (
                ("autonomous_delivery", "merge_policy", "default_merge_method"),
                "squash",
            ),
            (
                ("autonomous_delivery", "merge_policy", "require_base_synced"),
                True,
            ),
            (
                (
                    "autonomous_delivery",
                    "merge_policy",
                    "require_merge_method_selected",
                ),
                True,
            ),
            (
                (
                    "autonomous_delivery",
                    "merge_policy",
                    "require_no_admin_bypass",
                ),
                True,
            ),
            (
                (
                    "autonomous_delivery",
                    "merge_policy",
                    "require_no_direct_main_push",
                ),
                True,
            ),
        ]
        for path, expected in cases:
            with self.subTest(path=".".join(path), mutation="delete"):
                mutated_goal = copy.deepcopy(self.active_goal)
                owner = mutated_goal
                for segment in path[:-1]:
                    owner = owner[segment]
                owner.pop(path[-1])
                with self.assertRaises(ValueError):
                    self.validate_delivery(mutated_goal)

            with self.subTest(path=".".join(path), mutation="tamper"):
                mutated_goal = copy.deepcopy(self.active_goal)
                owner = mutated_goal
                for segment in path[:-1]:
                    owner = owner[segment]
                owner[path[-1]] = tampered(expected)
                with self.assertRaises(ValueError):
                    self.validate_delivery(mutated_goal)

    def test_post_merge_states_reject_bypass_transitions(self) -> None:
        for state in self.autonomous_checker.CORRECTNESS_DELIVERY_OUTGOING:
            with self.subTest(state=state):
                mutated_goal = copy.deepcopy(self.active_goal)
                mutated_goal["autonomous_delivery"]["transitions"].append(
                    {
                        "name": f"bypass_{state}",
                        "from": [state],
                        "to": "issue_reconciled_with_evidence",
                        "required_permissions": [],
                        "required_evidence": [],
                        "allowed_actions": ["skip_required_gate"],
                    }
                )
                with self.assertRaises(ValueError):
                    self.validate_delivery(mutated_goal)

    def test_each_feature_train_critical_field_is_required_and_exact(self) -> None:
        expected = (
            self.performance_checker.FORWARD_MIGRATION_FEATURE_TRAIN_REQUIRED_FIELDS
        )
        for key, value in expected.items():
            with self.subTest(key=key, mutation="delete"):
                mutated_matrix = copy.deepcopy(self.matrix)
                mutated_matrix["forward_migration_feature_train_v1"].pop(key)
                with self.assertRaises(ValueError):
                    self.performance_checker.validate_forward_migration_feature_train(
                        mutated_matrix
                    )

            with self.subTest(key=key, mutation="tamper"):
                mutated_matrix = copy.deepcopy(self.matrix)
                mutated_matrix["forward_migration_feature_train_v1"][key] = tampered(value)
                with self.assertRaises(ValueError):
                    self.performance_checker.validate_forward_migration_feature_train(
                        mutated_matrix
                    )

    def test_each_bootstrap_contract_field_is_required_and_exact(self) -> None:
        sections = {
            "daemon_bootstrap_v1": (
                self.performance_checker.DAEMON_BOOTSTRAP_V1_REQUIRED_FIELDS
            ),
            "desktop_supervisor_v2": (
                self.performance_checker.DESKTOP_SUPERVISOR_V2_REQUIRED_FIELDS
            ),
            "runtime_capability_degradation_v1": (
                self.performance_checker.RUNTIME_CAPABILITY_DEGRADATION_V1_REQUIRED_FIELDS
            ),
        }
        for section, expected in sections.items():
            for key, value in expected.items():
                with self.subTest(section=section, key=key, mutation="delete"):
                    mutated_matrix = copy.deepcopy(self.matrix)
                    mutated_matrix[section].pop(key)
                    with self.assertRaises(ValueError):
                        self.performance_checker.validate_matrix(mutated_matrix)

                with self.subTest(section=section, key=key, mutation="tamper"):
                    mutated_matrix = copy.deepcopy(self.matrix)
                    mutated_matrix[section][key] = tampered(value)
                    with self.assertRaises(ValueError):
                        self.performance_checker.validate_matrix(mutated_matrix)

if __name__ == "__main__":
    unittest.main()
