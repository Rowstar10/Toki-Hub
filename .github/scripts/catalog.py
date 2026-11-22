"""CI checks on the catalog's games, with the GitHub CLI (GH_TOKEN set):

    python .github/scripts/catalog.py check
        Every Android build listed in a game.json is published, and each game published as releases
        has all its files on its newest release: phones and computers are never offered an Install
        that can't work.
    python .github/scripts/catalog.py android-package hu-taong
        The Android package of the game's newest published Android build (empty if there's none),
        for the emulator test to install through the hub.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

CATALOG = Path(__file__).resolve().parents[2] / "catalog"


def gh_json(*args):
    result = subprocess.run(["gh", *args], capture_output=True, text=True)
    return json.loads(result.stdout) if result.returncode == 0 and result.stdout.strip() else None


def version_key(version):
    return [int(n) for n in re.findall(r"\d+", version)]


def fill(pattern, version):
    return pattern.replace("{version_}", re.sub(r"[.-]", "_", version)).replace("{version}", version)


def released(releases):
    """The game's published versions, newest first: (version, tag, asset names)."""
    prefix, _, suffix = releases.get("tag", "v{version}").partition("{version}")
    found = []
    for release in gh_json("api", f"repos/{releases['repo']}/releases?per_page=100") or []:
        tag = release["tag_name"]
        if release.get("draft") or not tag.startswith(prefix) or not tag.endswith(suffix):
            continue
        version = tag[len(prefix):len(tag) - len(suffix)] if suffix else tag[len(prefix):]
        if version:
            found.append((version, tag, {asset["name"] for asset in release.get("assets", [])}))
    return sorted(found, key=lambda r: version_key(r[0]), reverse=True)


def games():
    for path in sorted(CATALOG.glob("*/game.json")):
        yield path.parent.name, json.loads(path.read_text(encoding="utf-8"))


def check():
    problems = 0
    for game_id, game in games():
        for version in game.get("versions", []):
            for build in version.get("builds", []):
                source = build.get("source", {})
                if build.get("platform") == "android" and source.get("type") == "github-release":
                    view = gh_json("release", "view", source["tag"], "--repo", source["repo"], "--json", "assets") or {}
                    if source["asset"] in {a["name"] for a in view.get("assets", [])}:
                        print(f"ok: {game_id} {version['version']} for Android ({source['asset']})")
                    else:
                        print(f"::error file=catalog/{game_id}/game.json::{game_id} {version['version']} for Android: "
                              f"{source['repo']} has no {source['asset']} on {source['tag']}")
                        problems += 1
        releases = game.get("releases")
        if releases:
            found = released(releases)
            if not found:
                print(f"::error file=catalog/{game_id}/game.json::{game_id}: {releases['repo']} has no releases")
                problems += 1
                continue
            version, tag, assets = found[0]
            for build in releases.get("builds", []):
                asset = fill(build["asset"], version)
                if asset in assets:
                    print(f"ok: {game_id} {version} for {build['platform']} ({asset}, {len(found)} versions released)")
                else:
                    print(f"::error file=catalog/{game_id}/game.json::{game_id} {version}: {releases['repo']} "
                          f"has no {asset} on {tag}")
                    problems += 1
    sys.exit(1 if problems else 0)


def android_package(game_id):
    game = json.loads((CATALOG / game_id / "game.json").read_text(encoding="utf-8"))
    releases = game.get("releases")
    if releases:
        build = next((b for b in releases.get("builds", []) if b["platform"] == "android"), None)
        for version, _, assets in released(releases) if build else []:
            if fill(build["asset"], version) in assets:
                print(fill(build["launch"]["package"], version))
                return
    for version in game.get("versions", []):
        for build in version.get("builds", []):
            if build.get("platform") == "android":
                source = build["source"]
                view = gh_json("release", "view", source["tag"], "--repo", source["repo"], "--json", "assets") or {}
                if source["asset"] in {a["name"] for a in view.get("assets", [])}:
                    print(build["launch"]["package"])
                return


if __name__ == "__main__":
    if sys.argv[1:2] == ["check"]:
        check()
    elif sys.argv[1:2] == ["android-package"] and len(sys.argv) == 3:
        android_package(sys.argv[2])
    else:
        sys.exit(__doc__)
