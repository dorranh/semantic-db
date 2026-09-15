#!/usr/bin/env python3
"""Package prebuilt CLI/server, generated guides, and installable skills."""
import argparse
import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path("target/dist"))
    args = parser.parse_args()
    for label in (args.target, args.version):
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._+-]*", label):
            parser.error("target and version must be simple release labels")
    root = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    binary_version = args.version.removeprefix("v")
    suffix = ".exe" if "windows" in args.target else ""
    name = f"semantic-db-{args.version}-{args.target}"
    with tempfile.TemporaryDirectory(prefix="semantic-package-") as temporary:
        stage = Path(temporary) / name
        stage.mkdir()
        for binary in ("semantic-db", "semantic-server"):
            source = args.bin_dir.resolve() / f"{binary}{suffix}"
            result = subprocess.run([str(source), "--version"], check=True, capture_output=True, text=True)
            if result.stdout.strip().split()[-1] != binary_version:
                raise SystemExit(f"{binary} version does not match {args.version}: {result.stdout.strip()}")
            shutil.copy2(source, stage / source.name)
        for file in ("LICENSE",):
            shutil.copy2(root / file, stage / file)
        # Keep documentation paths stable inside the release archive.
        guides = stage / "docs"
        guides.mkdir()
        for file in (
            "delivery.md", "file-connectors.md", "release-quickstart.md",
            "writes-and-reconciliation-implementation.md",
            "writes-and-reconciliation-strategy.md",
            "writes-and-reconciliation-design.md",
        ):
            shutil.copy2(root / "docs/generated" / file, guides / file)
        shutil.copytree(root / "docs/generated/skills", stage / "skills")
        if suffix:
            archive = output / f"{name}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as handle:
                for file in sorted(stage.rglob("*")):
                    if file.is_file():
                        handle.write(file, file.relative_to(stage.parent))
        else:
            archive = output / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as handle:
                handle.add(stage, arcname=name)
        digest = hashlib.sha256()
        with archive.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        archive.with_name(archive.name + ".sha256").write_text(f"{digest.hexdigest()}  {archive.name}\n")
        print(archive)


if __name__ == "__main__":
    main()
