# Step 2 result: JSDOM answers versus the real browser's answers

Method: read the same fields twice, once in the existing JSDOM worker and
once in a blank tab of the user's saved Chrome (CDP, read-only evaluation,
tab closed after). No cookies, no storage contents, no tokens, no login.

Field-by-field comparison:

| field | JSDOM worker | real Chrome | mismatch |
|---|---|---|---|
| userAgent | `...jsdom/26.1.0` | `...Chrome/155.0.0.0...` | YES, fatal |
| platform | `` (empty) | `Linux x86_64` | YES |
| vendor | `Apple Computer, Inc.` | `Google Inc.` | YES |
| webdriver | absent/null | `false` | YES |
| screen.width | 0 | 1440 | YES |
| screen.height | 0 | 1000 | YES |
| screen.availWidth | 0 | 1440 | YES |
| screen.availHeight | 0 | 1000 | YES |
| outerWidth/Height | 1024x768 | matches inner | plausible |
| canvas getContext | throws 19x | draws fine | YES, fatal |
| webgl | null | null | same |
| userAgentData | absent | absent | same |
| hardwareConcurrency | 4 | 4 | same |
| languages | en-US, en | en-US, en | same |
| deviceMemory | null | null | same |
| colorDepth | 24 | 24 | same |

Real-browser capture: `real-browser-values.json`.

## What this means in plain words

The JSDOM token does not need to be "detected." It introduces itself.
Its user agent literally contains the string `jsdom/26.1.0`. Its platform
is blank, its vendor says Apple on a Linux machine, and its screen is 0 by
0 pixels. Any one of those is enough for a fraud check to refuse it.
Nineteen canvas exceptions on top make it certain.

This also explains why the earlier "which signal is wrong" question was
hard: it is not one signal. At least eight independent fields contradict
the claimed browser. Fixing one (for example installing the canvas npm
package) still leaves a token that calls itself jsdom.

## What the no-JSDOM realm must do

Replay every row of the right-hand column from `real-browser-values.json`:
Chrome 155 user agent, `Linux x86_64`, `Google Inc.`, webdriver false,
screen 1440x1000, working 2D canvas whose drawing bytes match the real
browser's `canvas2d` output. None of these values is invented; all were
copied from the user's own Chrome this session.

Canvas is the hard half. The realm needs a real rasterizer (node-canvas,
already pinned in the worker's optional bundle) plus a byte-comparison
against the captured `canvas2d` data URL. If the bytes differ, the font
rasterization differs and that probe still fails.

Fonts found in the real browser: Arial, Courier New, Times New Roman.
Those three must measure identically in the realm.
