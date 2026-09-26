#!/usr/bin/env python3
"""Offline checks for the decisions that must block a release."""

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "release_eligibility", Path(__file__).with_name("check-release-eligibility.py")
)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseEligibilityTests(unittest.TestCase):
    def test_only_matching_tag_or_main_dry_run_is_accepted(self):
        self.assertEqual(release.release_tag("push", "refs/tags/v0.1.0", "0.1.0"), "v0.1.0")
        self.assertEqual(release.release_tag("workflow_dispatch", "refs/heads/main", "0.1.0"), "v0.1.0")
        for event, ref in (
            ("push", "refs/tags/v9.9.9"),
            ("push", "refs/tags/v0.1.0-extra"),
            ("workflow_dispatch", "refs/heads/feature"),
        ):
            with self.subTest(event=event, ref=ref), self.assertRaises(ValueError):
                release.release_tag(event, ref, "0.1.0")

    def test_all_five_jobs_must_pass_on_the_same_main_push_commit(self):
        sha = "a" * 40
        run = {
            "id": 42,
            "event": "push",
            "head_branch": "main",
            "head_sha": sha,
            "status": "completed",
            "conclusion": "success",
        }
        jobs = [{"name": name, "conclusion": "success"} for name in release.REQUIRED_JOBS]
        self.assertEqual(release.successful_main_run([run], {42: jobs}, sha), 42)
        for bad_run in (
            {**run, "event": "pull_request"},
            {**run, "head_sha": "b" * 40},
            {**run, "conclusion": "failure"},
        ):
            with self.subTest(run=bad_run), self.assertRaises(ValueError):
                release.successful_main_run([bad_run], {42: jobs}, sha)
        with self.assertRaises(ValueError):
            release.successful_main_run([run], {42: jobs[:-1]}, sha)
        with self.assertRaises(ValueError):
            release.successful_main_run([run], {42: [*jobs[:-1], {**jobs[-1], "conclusion": "failure"}]}, sha)

    def test_existing_release_and_missing_main_ci_block_publication(self):
        sha = "a" * 40
        run = {
            "id": 42,
            "event": "push",
            "head_branch": "main",
            "head_sha": sha,
            "status": "completed",
            "conclusion": "success",
        }
        jobs = [{"name": name, "conclusion": "success"} for name in release.REQUIRED_JOBS]
        with tempfile.TemporaryDirectory() as directory:
            environment = {
                "GITHUB_EVENT_NAME": "push",
                "GITHUB_REF": "refs/tags/v0.1.0",
                "GITHUB_SHA": sha,
                "GITHUB_OUTPUT": str(Path(directory) / "output"),
            }
            with (
                patch.dict(os.environ, environment),
                patch.object(release, "git", return_value=sha),
                patch.object(release, "package_version", return_value="0.1.0"),
                patch.object(release.subprocess, "run", return_value=SimpleNamespace(returncode=0)),
            ):
                with patch.object(release, "api", return_value={"tag_name": "v0.1.0"}):
                    with self.assertRaisesRegex(ValueError, "already exists"):
                        release.main()
                with patch.object(release, "api", side_effect=[None, {"workflow_runs": []}]):
                    with self.assertRaisesRegex(ValueError, "no successful main push"):
                        release.main()
                with patch.object(
                    release,
                    "api",
                    side_effect=[None, {"workflow_runs": [run]}, {"total_count": 5, "jobs": jobs}],
                ):
                    release.main()
                self.assertIn("tag=v0.1.0", Path(environment["GITHUB_OUTPUT"]).read_text())


if __name__ == "__main__":
    unittest.main()
