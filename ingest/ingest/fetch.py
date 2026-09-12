"""Sparse git checkouts and release asset downloads."""

from __future__ import annotations

import hashlib
import logging
import subprocess
from pathlib import Path

import httpx

from ingest.models import ResolvedVersion

log = logging.getLogger(__name__)


class FetchError(RuntimeError):
    pass


def _run(args: list[str], cwd: Path | None = None) -> str:
    proc = subprocess.run(args, cwd=cwd, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        raise FetchError(f"{' '.join(args)}\n{proc.stderr.strip()}")
    return proc.stdout


def ref_exists(repo: str, ref: str) -> bool:
    out = _run(["git", "ls-remote", "--heads", "--tags", f"https://github.com/{repo}.git", ref])
    return bool(out.strip())


def resolve_ref(repo: str, template: str, version: ResolvedVersion) -> str:
    """Render `a|b|c` fallback template, return first ref that exists remotely."""
    candidates = [version.render(t.strip()) for t in template.split("|") if t.strip()]
    for ref in candidates:
        if ref_exists(repo, ref):
            return ref
    raise FetchError(f"{repo}: none of {candidates} exist")


def checkout(
    repo: str,
    ref_template: str,
    version: ResolvedVersion,
    work_dir: Path,
    sparse_paths: list[str],
) -> Path:
    ref = resolve_ref(repo, ref_template, version)
    key = hashlib.sha1(f"{repo}@{ref}".encode(), usedforsecurity=False).hexdigest()[:12]
    dest = work_dir / "repos" / f"{repo.replace('/', '__')}@{key}"
    url = f"https://github.com/{repo}.git"
    if not (dest / ".git").exists():
        dest.parent.mkdir(parents=True, exist_ok=True)
        log.info("clone %s@%s -> %s", repo, ref, dest)
        _run(
            [
                "git",
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--filter=blob:none",
                "--sparse",
                "--branch",
                ref,
                url,
                str(dest),
            ]
        )
        (dest / ".kd-ref").write_text(ref)
    # union of sparse paths across calls (idempotent)
    existing = (
        (dest / ".kd-sparse").read_text().split("\n") if (dest / ".kd-sparse").exists() else []
    )
    wanted = sorted({*existing, *sparse_paths} - {""})
    if wanted != sorted(set(existing) - {""}):
        _run(["git", "sparse-checkout", "set", "--no-cone", *wanted], cwd=dest)
        (dest / ".kd-sparse").write_text("\n".join(wanted))
    return dest


def sparse_dir(path_glob: str) -> str:
    """Turn a glob like 'a/b/**/*.yaml' into a sparse-checkout dir pattern 'a/b/'."""
    parts: list[str] = []
    for p in path_glob.split("/"):
        if any(ch in p for ch in "*?["):
            break
        parts.append(p)
    if len(parts) == len(path_glob.split("/")):
        return "/" + path_glob  # concrete file
    return "/" + "/".join(parts) + "/" if parts else "/*"


def download_release_asset(repo: str, tag: str, asset: str, work_dir: Path) -> Path:
    dest = work_dir / "assets" / repo.replace("/", "__") / tag / asset
    if dest.exists():
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    url = f"https://github.com/{repo}/releases/download/{tag}/{asset}"
    with httpx.stream("GET", url, follow_redirects=True, timeout=120.0) as r:
        if r.status_code != 200:
            raise FetchError(f"{url}: HTTP {r.status_code}")
        with dest.open("wb") as fh:
            for chunk in r.iter_bytes():
                fh.write(chunk)
    return dest
