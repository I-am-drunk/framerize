// Build one classic script from the six pinned modules so the graph runs in
// a vm context without the ES-module realm record. Imports/exports become a
// local module table. The wrapper's dynamic import of v3 becomes synchronous.
import fs from 'node:fs';

export function buildGraphScript({ assets, modules, cdn, collectionTransform }) {
  const roles = ['timeout', 'services', 'logger', 'bundler', 'v3', 'wrapper'];
  const cdnToRole = {};
  for (const [role, [name]] of Object.entries(modules)) cdnToRole[cdn + name] = role;
  let table = 'globalThis.__modules = {};\n';
  for (const role of roles) {
    const [name] = modules[role];
    let src;
    if (role === 'v3' && collectionTransform) {
      src = collectionTransform(fs.readFileSync(assets + '/' + name));
    } else {
      src = fs.readFileSync(assets + '/' + name, 'utf8');
    }
    // Rewrite imports to __modules lookups, exports to __modules assignments.
    src = rewriteModule(src, role, cdnToRole);
    table += `globalThis.__modules[${JSON.stringify(role)}] = (() => {\n${src}\n})();\n`;
  }
  return table;
}

function rewriteModule(src, role, cdnToRole) {
  // import { a as x, b as y } from "spec";
  src = src.replace(/import\s*\{([^}]*)\}\s*from\s*["']([^"']+)["'];?/g, (m, named, spec) => {
    const dep = cdnToRole[spec];
    if (!dep) throw new Error('unknown import ' + spec);
    const binds = named.split(',').map(b => {
      const [orig, alias] = b.trim().split(/\s+as\s+/);
      return `const ${alias || orig} = __modules[${JSON.stringify(dep)}].${orig};`;
    }).join('\n');
    return binds;
  });
  // Dynamic import("spec") -> Promise.resolve(__modules.role)
  src = src.replace(/import\s*\(\s*["']([^"']+)["']\s*\)/g, (m, spec) => {
    const dep = cdnToRole[spec];
    if (!dep) throw new Error('unknown dynamic import ' + spec);
    return `Promise.resolve(__modules[${JSON.stringify(dep)}])`;
  });
  // export { a as b, c as d } -> return { b: a, d: c }
  src = src.replace(/export\s*\{([^}]*)\};?\s*$/m, (m, named) => {
    const binds = named.split(',').map(b => {
      const [orig, alias] = b.trim().split(/\s+as\s+/);
      return `${alias || orig}: ${orig}`;
    }).join(', ');
    return `return { ${binds} };`;
  });
  return src;
}
