#!/usr/bin/env python3
"""Fail closed unless this commit and tag are eligible for a Linux release."""

import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path


REQUIRED_JOBS = {
    "Rust 1.86 / ubuntu-24.04",
    "Rust 1.86 / windows-2025",
    "Rust 1.86 / macos-15",
    "Rust 1.86 / macos-15-intel",
    "Rust stable / ubuntu-24.04",
}
TAG_PATTERN = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+\Z")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def release_tag(event: str, ref: str, version: str) -> str:
    if event == "push":
        require(ref.startswith("refs/tags/"), "release push must be a tag")
        tag = ref.removeprefix("refs/tags/")
    elif event == "workflow_dispatch":
        require(ref == "refs/heads/main", "dry run must use main")
        tag = f"v{version}"
    else:
        raise ValueError(f"unsupported event: {event}")
    require(TAG_PATTERN.fullmatch(tag) is not None, f"invalid release tag: {tag}")
    require(tag == f"v{version}", f"tag {tag} does not match pomodoro-tui v{version}")
    return tag


def successful_main_run(runs: list[dict], jobs_by_run: dict[int, list[dict]], sha: str) -> int:
    for run in runs:
        if not (
            run.get("event") == "push"
            and run.get("head_branch") == "main"
            and run.get("head_sha") == sha
            and run.get("status") == "completed"
            and run.get("conclusion") == "success"
        ):
            continue
        jobs = jobs_by_run.get(run["id"], [])
        passed = {job["name"] for job in jobs if job.get("conclusion") == "success"}
        if REQUIRED_JOBS <= passed:
            return run["id"]
    raise ValueError("no successful main push run with all five required jobs for this commit")


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True).strip()


def package_version() -> str:
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            text=True,
        )
    )
    return next(package["version"] for package in metadata["packages"] if package["name"] == "pomodoro-tui")


def api(path: str, *, missing_ok: bool = False) -> dict | None:
    repo = os.environ["GITHUB_REPOSITORY"]
    request = urllib.request.Request(
        f"https://api.github.com/repos/{repo}/{path}",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if missing_ok and error.code == 404:
            return None
        raise


def main() -> None:
    event = os.environ["GITHUB_EVENT_NAME"]
    ref = os.environ["GITHUB_REF"]
    sha = os.environ["GITHUB_SHA"]
    require(git("rev-parse", "HEAD") == sha, "checkout does not match workflow commit")
    version = package_version()
    tag = release_tag(event, ref, version)
    require(Path(f"docs/releases/{tag}.md").is_file(), f"missing release notes for {tag}")
    require(
        subprocess.run(["git", "merge-base", "--is-ancestor", sha, "origin/main"], check=False).returncode == 0,
        "release commit is not in main",
    )
    if event == "push":
        require(git("rev-parse", f"refs/tags/{tag}^{{commit}}") == sha, "tag points to another commit")
    require(api(f"releases/tags/{tag}", missing_ok=True) is None, f"Release {tag} already exists")

    response = api(
        f"actions/workflows/platform-probe.yml/runs?event=push&branch=main&head_sha={sha}&per_page=100"
    )
    runs = response["workflow_runs"]
    jobs_by_run = {}
    for run in runs:
        if run.get("head_sha") == sha and run.get("conclusion") == "success":
            jobs_response = api(f"actions/runs/{run['id']}/jobs?per_page=100")
            require(jobs_response["total_count"] <= 100, "workflow has too many jobs to verify")
            jobs_by_run[run["id"]] = jobs_response["jobs"]
    run_id = successful_main_run(runs, jobs_by_run, sha)
    print(f"eligible: {tag} at {sha}; main CI run {run_id}")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"tag={tag}\nmain_run_id={run_id}\n")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError, StopIteration, subprocess.CalledProcessError, ValueError) as error:
        print(f"release eligibility failed: {error}", file=sys.stderr)
        sys.exit(1)
