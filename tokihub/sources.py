"""Turning a build's `source` into something the installer can fetch.

`resolve()` returns either a Download (fetch a URL) or a LocalFiles (copy from this computer).
GitHub sources go through the GitHub API, so they work for private repositories as long as a
token is set in Settings (or the GITHUB_TOKEN environment variable).
"""
import json
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

from tokihub import __version__
from tokihub.errors import HubError
from tokihub.paths import safe_name

USER_AGENT = f"TokiHub/{__version__}"
GITHUB_API = "https://api.github.com"


@dataclass
class Download:
    url: str
    filename: str
    headers: dict = field(default_factory=dict)       # sent on every request, redirects included
    auth_headers: dict = field(default_factory=dict)  # sent to the first host only, never on redirects
    sha256: str = ""
    github_what: str = ""                             # set for GitHub downloads, for error messages


@dataclass
class LocalFiles:
    path: Path
    sha256: str = ""


class SourceResolver:
    def __init__(self, token_getter=lambda: "", github_api=GITHUB_API, timeout=30):
        self.token_getter = token_getter
        self.github_api = github_api.rstrip("/")
        self.timeout = timeout

    def resolve(self, source, game_folder):
        kind = source["type"]
        if kind == "url":
            name = source.get("filename") or _filename_from_url(source["url"])
            return Download(url=source["url"], filename=name, sha256=source.get("sha256", ""))
        if kind == "local":
            path = Path(source["path"]).expanduser()
            if not path.is_absolute():
                path = Path(game_folder) / path
            if not path.exists():
                raise HubError(f"The local files for this build are not on this computer: {path}")
            return LocalFiles(path=path, sha256=source.get("sha256", ""))
        if kind == "github-archive":
            repo, ref = source["repo"], source["ref"]
            return Download(url=f"{self.github_api}/repos/{repo}/zipball/{urllib.parse.quote(ref, safe='')}",
                            filename=f"{repo.split('/')[1]}-{_short(ref)}.zip",
                            headers={"Accept": "application/vnd.github+json"},
                            auth_headers=self._auth(), github_what=f"source code of {repo} at '{ref}'")
        if kind == "github-release":
            return self._release_asset(source)
        raise HubError(f"Unknown source type '{kind}'.")

    def _auth(self):
        token = self.token_getter()
        return {"Authorization": f"Bearer {token}"} if token else {}

    def _release_asset(self, source):
        repo, tag, asset_name = source["repo"], source["tag"], source["asset"]
        url = f"{self.github_api}/repos/{repo}/releases/tags/{urllib.parse.quote(tag, safe='')}"
        request = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json",
                                                       "User-Agent": USER_AGENT})
        for key, value in self._auth().items():
            request.add_unredirected_header(key, value)
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                release = json.load(response)
        except urllib.error.HTTPError as e:
            raise github_error(e, f"release '{tag}' of {repo}") from None
        except (urllib.error.URLError, OSError) as e:
            raise HubError(f"Could not reach GitHub: {getattr(e, 'reason', e)}") from None
        for asset in release.get("assets", []):
            if asset.get("name") == asset_name:
                return Download(url=asset["url"], filename=asset_name,
                                headers={"Accept": "application/octet-stream"},
                                auth_headers=self._auth(), sha256=source.get("sha256", ""),
                                github_what=f"file '{asset_name}' in release '{tag}' of {repo}")
        names = ", ".join(a.get("name", "?") for a in release.get("assets", [])) or "none"
        raise HubError(f"Release '{tag}' of {repo} has no file named '{asset_name}' (it has: {names}).")


def github_error(error, what):
    if error.code in (403, 429) and error.headers.get("X-RateLimit-Remaining") == "0":
        return HubError("GitHub's rate limit was reached. Add a GitHub token in Settings, or try again later.")
    if error.code == 401:
        return HubError("GitHub rejected the token in Settings. Check that it is correct and not expired.")
    if error.code in (403, 404):
        return HubError(f"GitHub could not find the {what}. If the repository is private, add a GitHub "
                        "token with access to it in Settings.")
    return HubError(f"GitHub returned an error for the {what}: HTTP {error.code} {error.reason}")


def _filename_from_url(url):
    name = Path(urllib.parse.unquote(urllib.parse.urlparse(url).path)).name
    return safe_name(name) if name else "download"


def _short(ref):
    return ref[:10] if len(ref) == 40 else ref.replace("/", "-")
