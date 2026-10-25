"""Build a small native differential fixture with the locked number formatter."""
import os
import json
import shutil
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


def compile_collation_fixture(source, directory):
    """Compile collection support with exactly the workspace's locked crates."""
    root = Path(__file__).resolve().parents[1]
    packages = tomllib.loads((root / "Cargo.lock").read_text())["package"]
    names = ["icu_collator", "icu_locale_core", "ryu-js", "serde_json"]
    versions = {name: next(p["version"] for p in packages if p["name"] == name) for name in names}
    manifest = directory / "Cargo.toml"
    lines = ['[package]', 'name="native-collation-fixture"', 'version="0.1.0"',
             'edition="2021"', '[[bin]]', 'name="native-collation-fixture"',
             f'path={json.dumps(str(source))}', '[dependencies]']
    lines += [f'{name}="={version}"' for name, version in versions.items()]
    manifest.write_text("\n".join(lines + ['[workspace]', '']))
    shutil.copyfile(root / "Cargo.lock", directory / "Cargo.lock")
    env = {**os.environ, "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0"}
    cargo = [os.environ.get("CARGO", "cargo"), "--manifest-path", str(manifest)]
    build = [cargo[0], "build", *cargo[1:], "--offline", "-j", "1"]
    subprocess.run(build, check=True, timeout=120, env=env)
    locked = {(p["name"], p["version"]): p.get("checksum") for p in packages}
    for package in tomllib.loads((directory / "Cargo.lock").read_text())["package"]:
        if package["name"] != "native-collation-fixture":
            assert locked[(package["name"], package["version"])] == package.get("checksum")
    tests = [cargo[0], "test", *cargo[1:], "--offline", "--locked", "-j", "1",
             "collection::tests", "--", "--test-threads=1"]
    result = subprocess.run(tests, check=True, capture_output=True, text=True, timeout=60, env=env)
    print(result.stdout, end="")
    assert "4 passed; 0 failed; 0 ignored" in result.stdout, result.stdout
    target = Path(env.get("CARGO_TARGET_DIR", directory / "target"))
    return target / "debug/native-collation-fixture"
