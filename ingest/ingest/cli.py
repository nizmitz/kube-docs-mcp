"""`ingest` command line."""

from __future__ import annotations

import dataclasses
import json
import os
import sqlite3
import sys
from datetime import UTC, datetime
from pathlib import Path

import typer
from rich.console import Console
from rich.table import Table

from ingest import registry as reg
from ingest.build import build_project, setup_logging
from ingest.db import merge_shards
from ingest.manifest import attribution as _attribution
from ingest.manifest import write_release
from ingest.versions import resolve

app = typer.Typer(no_args_is_help=True, add_completion=False)
console = Console()


@app.callback()
def _main(verbose: bool = typer.Option(False, "--verbose", "-v")) -> None:
    setup_logging(verbose)


@app.command("list")
def list_cmd(slugs: bool = typer.Option(False, "--slugs", help="one slug per line")) -> None:
    """List registry projects."""
    projects = reg.load_all()
    if slugs:
        for p in projects:
            typer.echo(p.slug)
        return
    t = Table("slug", "name", "category", "repo", "strategy", "schemas", "docs", "annotations")
    for p in projects:
        t.add_row(
            p.slug,
            p.name,
            p.category,
            p.source.repo,
            p.source.versions.strategy,
            str(len(p.schemas)),
            str(len(p.docs)),
            str(len(p.annotations)),
        )
    console.print(t)


@app.command()
def lint(remote: bool = typer.Option(False, help="also verify globs at the resolved ref")) -> None:
    """Validate all registry files against their JSON Schemas (+ pydantic)."""
    errors: list[str] = []
    files = sorted((reg.REGISTRY_DIR / "projects").glob("*.yaml")) + sorted(
        (reg.REGISTRY_DIR / "bootstrap").glob("*.yaml")
    )
    projects = []
    for f in files:
        try:
            projects.append(reg.load_project(f))
        except reg.RegistryError as e:
            errors.append(str(e))
    for f in sorted((reg.REGISTRY_DIR / "annotations").glob("*.yaml")):
        try:
            reg.load_curated(f)
        except reg.RegistryError as e:
            errors.append(str(e))
    if remote and not errors:
        from ingest.lint_remote import check_remote

        errors.extend(check_remote(projects, os.environ.get("GITHUB_TOKEN")))
    if errors:
        for msg in errors:
            console.print(f"[red]✗[/red] {msg}")
        raise typer.Exit(1)
    console.print(f"[green]✓[/green] {len(files)} project files valid")


@app.command("resolve")
def resolve_cmd(slug: str, work: Path = typer.Option(Path("work"))) -> None:
    """Print resolved versions for a project as JSON."""
    p = reg.find(slug)
    vs = resolve(p.source.repo, p.source.versions, work, os.environ.get("GITHUB_TOKEN"))
    typer.echo(json.dumps([v.model_dump() for v in vs], indent=2))


@app.command()
def build(
    slug: str,
    out: Path = typer.Option(..., "--out"),
    work: Path = typer.Option(Path("work"), "--work"),
    version: list[str] = typer.Option(None, "--version", help="git tag(s); overrides strategy"),
) -> None:
    """Build one project shard."""
    p = reg.find(slug)
    stats = build_project(p, work, out, version or None, os.environ.get("GITHUB_TOKEN"))
    typer.echo(json.dumps(dataclasses.asdict(stats)))


@app.command()
def merge(
    shards: list[Path] = typer.Option(..., "--shards"),
    out: Path = typer.Option(..., "--out"),
    ingest_sha: str = typer.Option("", "--ingest-sha"),
    registry_sha: str = typer.Option("", "--registry-sha"),
) -> None:
    """Merge shards into one index."""
    meta = {
        "build_date": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "ingest_sha": ingest_sha,
        "registry_sha": registry_sha,
    }
    merge_shards(shards, out, meta)
    conn = sqlite3.connect(f"file:{out}?mode=ro", uri=True)
    counts = {
        t: conn.execute(f"SELECT count(*) FROM {t}").fetchone()[0]  # noqa: S608
        for t in ("projects", "versions", "kinds", "fields", "doc_content", "docs", "annotations")
    }
    conn.close()
    typer.echo(json.dumps({"out": str(out), "size": out.stat().st_size, **counts}))


@app.command()
def manifest(
    index: Path = typer.Option(..., "--index"), out: Path = typer.Option(Path("dist"), "--out")
) -> None:
    """Write index.sqlite.zst, manifest.json, SHA256SUMS."""
    m = write_release(index, out)
    typer.echo(m.read_text())


@app.command("attribution")
def attribution_cmd(index: Path = typer.Option(..., "--index")) -> None:
    """Print ATTRIBUTION.md generated from the projects table."""
    sys.stdout.write(_attribution(index))


if __name__ == "__main__":
    app()
