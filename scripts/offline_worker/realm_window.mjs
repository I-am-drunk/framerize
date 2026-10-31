// Production-compatible window factory backed by the no-JSDOM realm.
// Satisfies every check OfflineCastleMinter performs on dom.window and
// dom.getInternalVMContext, so it is a drop-in for createOfflineWindow.
import { installSharedWorkers } from './shared_workers.mjs';
import { installDedicatedWorkers } from './dedicated_workers.mjs';
import { installWebGL } from './canvas_webgl.mjs';
import { buildRealmGlobals } from './realm_globals.mjs';
import vm from 'node:vm';

export function createRealmWindow(publicKey) {
  const globals = buildRealmGlobals({ publicKey });
  const context = vm.createContext(globals);
  const win = globals;
  if (typeof win.TextEncoder === 'undefined') win.TextEncoder = TextEncoder;
  win.requestAnimationFrame ??= cb => setTimeout(() => cb(performance.now()), 16);
  win.cancelAnimationFrame ??= id => clearTimeout(id);
  if (!win.document.hidden) win.document.hidden = false;
  win.document.visibilityState ??= 'visible';
  return { window: win, getInternalVMContext: () => context };
}
