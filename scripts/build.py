#!/usr/bin/env python3
"""Build selected native or Claviron HTTP hosts with the same release profile."""

import argparse
import json
import os
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]
NATIVE = ("tokio", "mio", "hyper", "axum")
CLAVIRON = ("claviron-other", "claviron-axum", "claviron-routable")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true", help="Optimize for this CPU")
    parser.add_argument("--target", choices=("all", "native", "claviron", *NATIVE, *CLAVIRON),
                        default="all", help="Select a group or one binary (default: all)")
    parser.add_argument("--supervisor", action="store_true",
                        help="Expose --workers on Claviron binaries")
    parser.add_argument("--claviron-source", type=Path,
                        help="Use a Claviron source checkout instead of the registry")
    args = parser.parse_args()
    targets = {"all": NATIVE + CLAVIRON, "native": NATIVE, "claviron": CLAVIRON}.get(args.target, (args.target,))
    with_claviron = any(target in CLAVIRON for target in targets)
    if args.supervisor and not with_claviron:
        parser.error("--supervisor requires a Claviron target")
    features = ["--features", "supervisor" if args.supervisor else "claviron"] if with_claviron else []
    env = os.environ.copy()
    if args.native:
        env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C target-cpu=native").strip()
    cargo = ["cargo"]
    if args.claviron_source:
        source = args.claviron_source.resolve()
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        config = cache / "source.toml"
        config.write_text(
            "[patch.crates-io]\nclaviron = { path = " + json.dumps(str(source)) + " }\n"
        )
        cargo += ["--config", str(config)]
    metadata = json.loads(subprocess.check_output(
        [*cargo, "metadata", "--format-version", "1", *features],
        cwd=ROOT, env=env, text=True,
    ))
    if with_claviron:
        resolved = next(p for p in metadata["packages"] if p["name"] == "claviron")
        if args.claviron_source:
            expected = source / "Cargo.toml"
            if resolved["source"] is not None or Path(resolved["manifest_path"]) != expected:
                raise SystemExit("Claviron did not resolve to the selected source checkout")
            revision = subprocess.check_output(
                ["git", "-C", str(source), "rev-parse", "HEAD"], text=True,
            ).strip()
            print("Claviron source revision: " + revision, flush=True)
        else:
            print("Claviron registry version: " + resolved["version"], flush=True)
    binaries = [argument for target in targets for argument in ("--bin", target)]
    subprocess.run(
        [*cargo, "build", "--release", "--workspace", *features, *binaries],
        cwd=ROOT, env=env, check=True,
    )
    print("Binary directory: " + str(Path(metadata["target_directory"]) / "release"))


if __name__ == "__main__":
    main()
