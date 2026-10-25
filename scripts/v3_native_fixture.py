"""Build a small native differential fixture with the locked number formatter."""
import os
from pathlib import Path
import subprocess
import tomllib


def compile_fixture(source, binary):
    root = Path(__file__).resolve().parents[1]
    packages = tomllib.loads((root / "Cargo.lock").read_text())["package"]
    package = next(p for p in packages if p["name"] == "ryu-js")
    dependency = os.environ.get("V3_RYU_SOURCE")
    if dependency is None:
        cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
        matches = list(cargo_home.glob(f'registry/src/*/ryu-js-{package["version"]}/src/lib.rs'))
        if len(matches) != 1:
            raise RuntimeError("Run cargo fetch --locked, or supply the isolated V3_RYU_SOURCE")
        dependency = str(matches[0])
    manifest = tomllib.loads((Path(dependency).parents[1] / "Cargo.toml").read_text())
    assert manifest["package"]["name"] == "ryu-js"
    assert manifest["package"]["version"] == package["version"]
    compiler = os.environ.get("RUSTC", "rustc")
    library = binary.parent / "libryu_js.rlib"
    subprocess.run([compiler, "--edition=2021", "--crate-name=ryu_js", "--crate-type=rlib",
                    dependency, "-o", str(library)], check=True, timeout=30)
    subprocess.run([compiler, "--edition=2021", "--test", str(source),
                    "--extern", f"ryu_js={library}", "-o", str(binary)],
                   check=True, timeout=30)
