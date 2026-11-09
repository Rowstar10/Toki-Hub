"""Ordering version strings the way a person expects: 1.10 > 1.9 > 1.0 > 1.0-beta."""
import re

_VERSION = re.compile(r"^v?(\d+(?:\.\d+)*)(.*)$", re.IGNORECASE)


def version_key(version):
    """Sort key for a version string. Use with reverse=True for newest first.

    Numbers compare as numbers, "1.0" equals "1.0.0", a release sorts after its own
    pre-releases ("1.0" > "1.0-rc2" > "1.0-beta"), and versions that do not start with a
    number sort below all numbered ones, alphabetically.
    """
    match = _VERSION.match(version.strip())
    if not match:
        return ((), 0, version.strip().lower())
    numbers = [int(part) for part in match.group(1).split(".")]
    while len(numbers) > 1 and numbers[-1] == 0:
        numbers.pop()
    suffix = match.group(2).strip(" -_.+").lower()
    return (tuple(numbers), 0 if suffix else 1, suffix)


def newest_first(versions, key=lambda v: v):
    return sorted(versions, key=lambda item: version_key(key(item)), reverse=True)
