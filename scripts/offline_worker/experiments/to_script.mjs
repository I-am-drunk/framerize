// Transpile the six pinned ES modules into one classic script that runs in
// a vm context without the ES-module realm record. Dependency order is
// fixed by pins.mjs; imports/exports become a local module table.
import fs from 'node:fs';
import { ASSETS, MODULES } from '../pins.mjs';

const ORDER = ['timeout', 'services', 'logger', 'bundler', 'v3', 'wrapper'];
// Map CDN specifier -> role
const CDN_TO_ROLE = {};
for (const [role, [name]] of Object.entries(MODULES))
  CDN_TO_ROLE['https://app.framerstatic.com/' + name] = role;

export function buildScript(collectionTransform) {
  let body = '';
  const exportNames = {};
  for (const role of ORDER) {
    const [name] = MODULES[role];
    let src = fs.readFileSync(ASSETS + '/' + name, 'utf8');
    if (role === 'v3' && collectionTransform) src = collectionTransform(src);
    // Strip import statements; record them.
    src = src.replace(/import\s*(?:\{([^}]*)\}|\*\s*as\s*(\w+)|(\w+))\s*from\s*["']([^"']+)["'];?/g,
      (m, named, star, def, spec) => {
        const dep = CDN_TO_ROLE[spec];
        if (!dep) throw new Error('unknown import ' + spec);
        return `/*import from ${dep}*/`;
      });
    body += `\n;//---MODULE ${role}---\n(function(){\n${src}\n})();\n`;
  }
  return body;
}
