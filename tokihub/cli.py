"""Command line:

    python main.py                   open the hub
    python main.py --browser         open the hub in your web browser instead of its own window
    python main.py check             check the catalog for mistakes
    python main.py new-game "Title"  start a catalog entry for a new game
"""
import argparse
import json
import re
import sys
from pathlib import Path

from tokihub import __version__, paths
from tokihub.catalog import Catalog


def main(argv=None):
    parser = argparse.ArgumentParser(prog="main.py", description="Toki Hub: launcher for all your games.")
    parser.add_argument("--version", action="version", version=f"Toki Hub {__version__}")
    parser.add_argument("--browser", action="store_true", help="open in your web browser instead of a window")
    parser.add_argument("--port", type=int, default=0, help="port for the hub's local server (default: any free one)")
    parser.add_argument("--no-open", action="store_true", help="with --browser: do not open a browser tab")
    parser.add_argument("--catalog", type=Path, help="use a different catalog folder")
    parser.add_argument("--data-dir", type=Path, help="use a different data folder (settings, installed games)")
    commands = parser.add_subparsers(dest="command", metavar="command")
    commands.add_parser("check", help="check the catalog for mistakes")
    new_game = commands.add_parser("new-game", help="start a catalog entry for a new game")
    new_game.add_argument("title", help='the game\'s name, e.g. "Hu Taong"')
    new_game.add_argument("--id", help="folder name / id (default: made from the title)")
    args = parser.parse_args(argv)

    if args.command == "check":
        return check(args.catalog or paths.catalog_dirs(args.data_dir))
    if args.command == "new-game":
        return new_game_entry(args.catalog or paths.catalog_dir(args.data_dir), args.title, args.id)

    from tokihub.app import run
    return run(browser=args.browser, port=args.port, open_browser=not args.no_open,
               data_dir=args.data_dir, catalog_dir=args.catalog)


def check(catalog_dirs):
    catalog = Catalog.load(catalog_dirs)
    print("Catalog: " + ", ".join(str(f) for f in catalog.folders if f.is_dir()))
    for game in catalog.games.values():
        print(f"\n  {game.title}  [{game.id}]")
        if not game.versions:
            print("    (no versions yet)")
        for version in game.versions:
            builds = ", ".join(f"{b.platform}: {b.source['type']} -> {b.launch['type']} {b.launch['path']}"
                               for b in version.builds) or "no builds"
            print(f"    {version.version:<12} {builds}")
    if catalog.problems:
        print(f"\n{len(catalog.problems)} problem(s):")
        for problem in catalog.problems:
            print(f"  - {problem}")
        return 1
    print(f"\nAll good: {len(catalog.games)} game(s), no problems.")
    return 0


def slugify(title):
    return re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")


def new_game_entry(catalog_dir, title, game_id=None):
    game_id = game_id or slugify(title)
    if not re.match(r"^[a-z0-9][a-z0-9-]*$", game_id or ""):
        print("Choose an id made of lowercase letters, digits and dashes with --id.", file=sys.stderr)
        return 2
    folder = Path(catalog_dir) / game_id
    if folder.exists():
        print(f"{folder} already exists.", file=sys.stderr)
        return 1
    folder.mkdir(parents=True)
    entry = {
        "title": title,
        "tagline": "",
        "description": "",
        "developer": "Rowstar10",
        "tags": [],
        "versions": [],
    }
    (folder / "game.json").write_text(json.dumps(entry, indent=2) + "\n", encoding="utf-8")
    print(f"Created {folder / 'game.json'}\n\nNext:\n"
          f"  1. Put icon.png (square) and cover.jpg (16:9) in {folder} and add \"icon\"/\"cover\" to game.json.\n"
          f"  2. Add a version to \"versions\" (catalog/README.md has copy-paste examples).\n"
          f"  3. Run: python main.py check")
    return 0
