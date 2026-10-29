#!/usr/bin/env python3
"""Build all native HTTP hosts with the same release profile and allocator."""

import argparse
import json
import os
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true", help="Optimize for this CPU")
    parser.add_argument("--supervisor", action="store_true",
                        help="Enable supervised worker processes for Bypass")
    parser.add_argument("--claviron-source", type=Path,
                        help="Use a Claviron source checkout instead of the registry")
    args = parser.parse_args()
    features = ["--features", "supervisor"] if args.supervisor else []
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
    subprocess.run(
        [*cargo, "build", "--release", "--workspace", *features, "--all-targets"],
        cwd=ROOT, env=env, check=True,
    )
    print("Binary directory: " + str(Path(metadata["target_directory"]) / "release"))


if __name__ == "__main__":
    main()
