// Adapted from T3 Code .github/scripts/ci-changes.cjs at 5a906583 (MIT).
// Unknown paths run every job; recognized packages pull in their dependents.
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "../..");

// Root and docs Markdown ship in no package. Package READMEs stay owned.
const INERT = [/^docs\//, /^\.agents\//, /^[^/]+\.md$/, /^LICENSE$/];
// These files cannot change the measured request path. A dependency test checks
// that the runtime entry points never import a skipped source or test file.
const PERFORMANCE_INERT = [...INERT,
  /^framerize-(?:core|remote|server)\/(?:test\/|.*\.md$)/,
  /^framerize-server\/src\/(?:cli|setup|service-control|service-units)\.ts$/,
  /^framerize-remote\/src\/(?:clientCli|serviceCli|serviceUnits|cliArgs|cmdSetup)\.ts$/,
];
// The server bundles core during its build without declaring it.
const BUILD_ONLY_DEPS = { framerize: ["@framerize/core"] };
const GATES = { source: null, packages: ["framerize-remote", "framerize"], server: ["framerize"] };

function readWorkspace() {
  const { workspaces } = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  return [...workspaces, "framerize-server"].map((dir) => {
    const manifest = JSON.parse(fs.readFileSync(path.join(root, dir, "package.json"), "utf8"));
    const deps = { ...manifest.dependencies, ...manifest.devDependencies, ...manifest.peerDependencies };
    const local = Object.keys(deps).filter((dep) => String(deps[dep]).startsWith("file:"));
    return { name: manifest.name, dir, deps: [...local, ...(BUILD_ONLY_DEPS[manifest.name] ?? [])] };
  });
}

// [] means inert; null means unknown and requires every job.
function ownersOf(file, packages) {
  if (INERT.some((pattern) => pattern.test(file))) return [];
  if (path.posix.basename(file) === "package.json") return null;
  const owner = packages.find((pkg) => file.startsWith(`${pkg.dir}/`));
  return owner ? [owner.name] : null;
}

function decide(files, packages, full) {
  let affected = new Set();
  let everything = full || files === null;
  for (const file of files ?? []) {
    const owners = ownersOf(file, packages);
    if (owners === null) {
      everything = true;
      console.error(`Runs everything: ${file}`);
    } else {
      for (const owner of owners) affected.add(owner);
    }
  }
  if (everything) affected = new Set(packages.map((pkg) => pkg.name));
  let grew = true;
  while (grew) {
    grew = false;
    for (const pkg of packages) {
      if (!affected.has(pkg.name) && pkg.deps.some((dep) => affected.has(dep))) {
        affected.add(pkg.name);
        grew = true;
      }
    }
  }
  const outputs = Object.fromEntries(Object.entries(GATES).map(([gate, names]) =>
    [gate, names === null ? affected.size > 0 : names.some((name) => affected.has(name))]));
  outputs.performance = everything || (files ?? []).some((file) =>
    !PERFORMANCE_INERT.some((pattern) => pattern.test(file)));
  return { affected: [...affected].sort(), outputs };
}

function main() {
  const args = process.argv.slice(2);
  const pathsIndex = args.indexOf("--paths");
  const files = pathsIndex === -1 ? null
    : fs.readFileSync(args[pathsIndex + 1], "utf8").split("\n").filter(Boolean);
  const { affected, outputs } = decide(files, readWorkspace(), args.includes("--full"));
  console.error(`Affected packages: ${affected.join(", ") || "none"}`);
  for (const [key, value] of Object.entries(outputs)) console.log(`${key}=${value}`);
}

module.exports = { readWorkspace, decide };

if (require.main === module) main();
