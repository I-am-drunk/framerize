import vm from 'node:vm';
const ctx = vm.createContext({ atob: globalThis.atob, btoa: globalThis.btoa, console });
const r = vm.runInContext('typeof atob', ctx);
console.log('typeof atob in ctx:', r);
const r2 = vm.runInContext('atob("aGVsbG8=")', ctx);
console.log('atob call:', r2);
// Now test inside a SourceTextModule
const mod = new vm.SourceTextModule('export default typeof atob;', { context: ctx });
await mod.link(() => { throw new Error('no imports'); });
await mod.evaluate();
console.log('typeof atob in module:', mod.namespace.default);
