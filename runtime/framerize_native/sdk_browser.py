RESIDENT_SDK_OBSERVER_JS = r"""async input => {
  const unavailable = () => null;
  try {
    if (input.wrapper_export !== 'a' || !/^https:\/\/app\.framerstatic\.com\/chunk-5LKSDTQ5\.mjs(?:\?cb=[0-9]{1,20})?$/.test(input.wrapper_url))
      return unavailable;
    // This exact wrapper statically imported this dependency in this realm.
    // Never change logging levels, buffer capacity, or SDK fallback behavior.
    const lm = await import('https://app.framerstatic.com/chunk-VJ7UYMJI.mjs');
    const logger = lm.i('castle');
    const valid = records => {
      if (!Array.isArray(records) || records.length > 1000) return false;
      let prior = -1;
      for (const r of records) {
        if (!r || !Number.isSafeInteger(r.id) || r.id <= prior) return false;
        prior = r.id;
      }
      return true;
    };
    const initial = lm.h();
    if (!valid(initial)) return unavailable;
    const anchor = initial.at(-1), since = anchor?.id ?? -1;
    return () => {
      const records = lm.h();
      if (!valid(records)) return null;
      const out = {complete: since < 0 || records.includes(anchor),
        v3: 0, v2: 0, missing: 0};
      const phases = {'module load': 'module_load', configuration: 'configuration',
        'token generation': 'token_generation', SDK: 'sdk'};
      for (const phase of Object.values(phases))
        for (const failure of ['timeout', 'error']) out[phase + '_' + failure] = 0;
      let last = since;
      for (const r of records) {
        if (r.id <= since) continue;
        if (r.id !== last + 1) out.complete = false;
        last = r.id;
        if (r.logger !== logger) continue;
        if (!Array.isArray(r.parts)) return null;
        if (r.parts[0] === 'Using Castle v3 token') out.v3++;
        if (r.parts[0] === 'Using Castle v2 token') out.v2++;
        if (r.parts[0] === 'No Castle token available') out.missing++;
        const tags = r.parts[2];
        if (tags?.castleVersion === 'v3' && Object.hasOwn(phases, tags.castlePhase)
            && ['timeout', 'error'].includes(tags.castleFailure))
          out[phases[tags.castlePhase] + '_' + tags.castleFailure]++;
      }
      return out;
    };
  } catch { return unavailable; }
}"""
