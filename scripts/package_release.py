"""Package only explicit release files from a tested native build (Python 3.11+)."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
)


def output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, timeout=30).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    args = parser.parse_args()
    if output("git", "status", "--porcelain", "--untracked-files=normal"):
        parser.error("release packaging requires a clean checkout of the source commit")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    rustc = output("rustc", "-vV")
    if f"host: {args.target}" not in rustc.splitlines():
        parser.error("target must match this runner's native Rust toolchain")
    suffix = ".exe" if args.target.endswith("windows-msvc") else ""
    binary_dir = args.bin_dir.resolve()
    if output(str(binary_dir / f"astral{suffix}"), "--version") != f"astral {version}":
        parser.error("built binary version differs from Cargo.toml")
    # Helpers must start successfully on the native runner too.
    for name in ("astral-state", "stats"):
        output(str(binary_dir / f"{name}{suffix}"), "--help")
    files = {}
    for name in ("astral", "astral-state", "stats"):
        files[f"{name}{suffix}"] = ((binary_dir / f"{name}{suffix}").read_bytes(), 0o755)
    for name in ("README.md", "LICENSE"):
        files[name] = ((ROOT / name).read_bytes(), 0o644)
    notes = ROOT / "docs/releases" / f"v{version}.md"
    files["RELEASE_NOTES.md"] = (notes.read_bytes(), 0o644)
    manifest = {
        "version": version,
        "target": args.target,
        "source_commit": output("git", "rev-parse", "HEAD"),
        "rustc": rustc,
        "files": {name: hashlib.sha256(data).hexdigest() for name, (data, _) in files.items()},
    }
    files["manifest.json"] = ((json.dumps(manifest, indent=2) + "\n").encode(), 0o644)
    prefix = f"astral-{version}-{args.target}"
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / (prefix + (".zip" if suffix else ".tar.gz"))
    # Exclusive creation prevents silently replacing a previously reviewed asset.
    if suffix:
        with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as bundle:
            for name, (data, mode) in files.items():
                entry = zipfile.ZipInfo(f"{prefix}/{name}")
                entry.external_attr = (0o100000 | mode) << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                bundle.writestr(entry, data)
    else:
        with archive.open("xb") as raw:
            with tarfile.open(fileobj=raw, mode="w:gz") as bundle:
                for name, (data, mode) in files.items():
                    entry = tarfile.TarInfo(f"{prefix}/{name}")
                    entry.mode = mode
                    entry.size = len(data)
                    bundle.addfile(entry, io.BytesIO(data))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    with archive.with_name(archive.name + ".sha256").open("x") as receipt:
        receipt.write(f"{digest}  {archive.name}\n")
    print(json.dumps({"archive": str(archive), "sha256": digest, **manifest}, indent=2))


if __name__ == "__main__":
    main()
