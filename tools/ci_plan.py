#!/usr/bin/env python3
"""Decide which apps need a new build (used by .github/workflows/build.yml).

An app is rebuilt when its upstream repository has new commits, when its port (this repo's
files for it) changed, or when the run asks for it. The last build of each app is its newest
GitHub release, whose notes record the upstream commit and the port revision it was built from:

    upstream: storytold/lightcraft@<sha>
    port: <sha>

Writes a JSON matrix to $GITHUB_OUTPUT as `matrix` and `any`.
"""
import json
import os
import re
import subprocess
import sys

APPS = {
    # key: (upstream repo or None, paths of this repo that make up the port)
    "lightcraft": ("storytold/lightcraft", ["lightcraft", "mobile-kit", "common"]),
    "filmcraft": ("storytold/filmcraft", ["filmcraft", "mobile-kit", "common"]),
    "launcher": (None, ["launcher"]),
}


def sh(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()


def upstream_head(repo, ref):
    out = sh("git", "ls-remote", f"https://github.com/{repo}", ref)
    for line in out.splitlines():
        sha, name = line.split("\t")
        if name in (ref, f"refs/heads/{ref}") or ref == "HEAD":
            return sha
    raise SystemExit(f"{repo}: no ref {ref}")


def port_rev(paths):
    return sh("git", "log", "-1", "--format=%H", "--", *paths) or sh("git", "rev-parse", "HEAD")


def last_build(key):
    """(upstream sha, port sha) of the newest release of `key`, or (None, None)."""
    try:
        rels = json.loads(sh("gh", "api", f"repos/{os.environ['GITHUB_REPOSITORY']}/releases?per_page=100"))
    except subprocess.CalledProcessError:
        return None, None
    best = None
    for r in rels:
        m = re.fullmatch(rf"{key}-b(\d+)", r.get("tag_name", ""))
        if m and not r.get("draft") and (best is None or int(m.group(1)) > best[0]):
            best = (int(m.group(1)), r.get("body") or "")
    if not best:
        return None, None
    up = re.search(r"upstream: \S+@([0-9a-f]{40})", best[1])
    port = re.search(r"port: ([0-9a-f]{40})", best[1])
    return (up.group(1) if up else None), (port.group(1) if port else None)


def main():
    want = os.environ.get("APPS", "all").strip() or "all"
    force = os.environ.get("FORCE", "false") == "true"
    ref = os.environ.get("UPSTREAM_REF", "").strip() or "HEAD"
    keys = list(APPS) if want == "all" else [k.strip() for k in want.split(",") if k.strip() in APPS]
    include = []
    for key in keys:
        repo, paths = APPS[key]
        up = upstream_head(repo, ref) if repo else ""
        port = port_rev(paths)
        last_up, last_port = last_build(key)
        reasons = []
        if force:
            reasons.append("asked")
        if repo and up != last_up:
            reasons.append(f"upstream {last_up[:7] if last_up else 'none'} -> {up[:7]}")
        if port != last_port:
            reasons.append("port changed")
        print(f"{key}: {', '.join(reasons) or 'up to date'}", file=sys.stderr)
        if reasons:
            include.append({"app": key, "upstream": repo or "", "sha": up, "port": port, "why": "; ".join(reasons)})
    with open(os.environ.get("GITHUB_OUTPUT", "/dev/stdout"), "a") as f:
        f.write(f"matrix={json.dumps({'include': include})}\n")
        f.write(f"any={'true' if include else 'false'}\n")


if __name__ == "__main__":
    main()
