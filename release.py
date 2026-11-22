"""Publish a version of Toki Hub on the public Toki-Hub-Releases repo, where every copy of Toki Hub
looks for updates and where people download it. Run it on your PC from this folder:

    py release.py

First: give the new version its section at the top of CHANGELOG.md, set the same version in
src-tauri/Cargo.toml, push, and wait for GitHub's "Build Toki Hub" run of that commit to pass (it
builds Toki Hub.exe and Toki-Hub-android.apk and tries both). This takes those two files from that
run and publishes them as release tauri-v<version>, with the CHANGELOG.md section as its notes.
It uses the GitHub CLI (cli.github.com) signed in as you ('gh auth login' once): no token is stored.
"""
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

RELEASES_REPO = "Rowstar10/Toki-Hub-Releases"
WORKFLOW = "build.yml"
BRANCH = "Toki-Hub"
# The build's artifacts, and the file each holds
ARTIFACTS = {"Toki Hub": "Toki Hub.exe", "Toki Hub for Android": "Toki-Hub-android.apk"}

ROOT = Path(__file__).resolve().parent
DIST = ROOT / "dist" / "release"


def say(text):
    print(f"\n== {text}", flush=True)


def stop(text):
    print(f"\nRelease stopped: {text}", flush=True)
    sys.exit(1)


def run(*args, capture=False):
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=capture, encoding="utf-8")
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip() if capture else ""
        stop(f"{' '.join(args[:3])} ... failed{': ' + detail if detail else ''}")
    return result.stdout.strip() if capture else ""


def version():
    text = (ROOT / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8")
    found = re.search(r'^version = "([^"]+)"', text, re.M)
    if not found:
        stop("no version in src-tauri/Cargo.toml")
    return found.group(1)


def changelog_section(version):
    lines, inside = [], False
    for line in (ROOT / "CHANGELOG.md").read_text(encoding="utf-8").splitlines():
        if line.startswith("## "):
            inside = line[3:].split()[:1] == [version]
            continue
        if inside:
            lines.append(line)
    text = "\n".join(lines).strip()
    if not text:
        stop(f"CHANGELOG.md has no section for {version}: add one at the top (like the others) and push it")
    return text


def main():
    if not shutil.which("gh"):
        stop("the GitHub CLI isn't installed. Get it from https://cli.github.com, run 'gh auth login' once, and try again.")
    if subprocess.run(["gh", "auth", "status"], capture_output=True).returncode != 0:
        stop("the GitHub CLI isn't signed in. Run 'gh auth login' once, then try again.")
    url = run("git", "remote", "get-url", "origin", capture=True)
    repo = re.search(r"github\.com[:/]+([^/]+/[^/.]+)", url).group(1)
    v = version()
    tag = f"tauri-v{v}"
    notes = changelog_section(v)

    run("git", "fetch", "--quiet", "origin", BRANCH)
    commit = run("git", "rev-parse", f"origin/{BRANCH}", capture=True)
    if run("git", "rev-parse", "HEAD", capture=True) != commit:
        stop(f"push your latest commit to {BRANCH} first (this folder isn't the same as GitHub's {BRANCH})")
    say(f"Toki Hub {v}, commit {commit[:10]}")

    runs = json.loads(run("gh", "run", "list", "--repo", repo, "--workflow", WORKFLOW, "--commit", commit,
                          "--limit", "5", "--json", "databaseId,status,conclusion", capture=True) or "[]")
    if not runs:
        stop("GitHub hasn't built this commit yet: wait a minute after pushing, then try again")
    build = runs[0]
    run_id = str(build["databaseId"])
    print(f"https://github.com/{repo}/actions/runs/{run_id}")
    if build["status"] != "completed":
        say("Waiting for GitHub's build to finish")
        if subprocess.run(["gh", "run", "watch", run_id, "--repo", repo, "--exit-status", "--interval", "30"]).returncode != 0:
            stop(f"the build failed: https://github.com/{repo}/actions/runs/{run_id}")
    elif build["conclusion"] != "success":
        stop(f"that build didn't pass ({build['conclusion']}): https://github.com/{repo}/actions/runs/{run_id}")

    say("Downloading what it built")
    shutil.rmtree(DIST, ignore_errors=True)
    files = []
    for artifact, name in ARTIFACTS.items():
        run("gh", "run", "download", run_id, "--repo", repo, "--name", artifact, "--dir", str(DIST))
        path = DIST / name
        if not path.exists():
            stop(f"the build's '{artifact}' has no {name}")
        files.append(path)

    say(f"Publishing {tag} on {RELEASES_REPO}")
    notes_file = DIST / "notes.md"
    notes_file.write_text(
        notes + "\n\n---\n\nOn Windows, download **Toki Hub.exe** below and double-click it: nothing to install. "
        "Windows may say it \"protected your PC\" (the exe isn't code-signed): click **More info**, then **Run anyway**. "
        "On an Android phone or tablet, download **Toki-Hub-android.apk** and open it. "
        "Copies already running offer this update by themselves.\n", encoding="utf-8")
    assets = [f"{files[0]}#Toki Hub.exe", str(files[1])]
    if subprocess.run(["gh", "release", "view", tag, "--repo", RELEASES_REPO], capture_output=True).returncode == 0:
        run("gh", "release", "upload", tag, *assets, "--repo", RELEASES_REPO, "--clobber")
    else:
        run("gh", "release", "create", tag, *assets, "--repo", RELEASES_REPO, "--title", f"Toki Hub {v}",
            "--notes-file", str(notes_file), "--latest")
    say(f"Toki Hub {v} is out: https://github.com/{RELEASES_REPO}/releases/tag/{tag}")


if __name__ == "__main__":
    main()
