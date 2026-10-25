# V3 stock shared-worker output

Source: SDK SHA256 `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
The stock source is the decoded literal `j(898,su,Vn,t)` in bytes 362804..364430.
Decoding with the original string decoder produced 920 bytes, SHA256
`cb1716096c87eff9d6f2296bbac1302c0b3c58e404226b15cd45cd223c4addab`.
Only the pure decoder ran, in the existing isolated reference environment.
The decoded worker was read as source and was not executed.

On a `start` message it constructs an array of length 13:

| Index | Source value |
| --- | --- |
| 0 | Success tag `2` |
| 1 | Literal `false` |
| 2 | `navigator.userAgent` |
| 3 | JSON of `userAgentData.brands`, or `NA` |
| 4 | `userAgentData.mobile` |
| 5 | `userAgentData.platform` |
| 6 | High-entropy `architecture`, or `NA` |
| 7 | High-entropy `bitness`, or `NA` |
| 8 | High-entropy `model`, or `NA` |
| 9 | High-entropy `platformVersion`, or `NA` |
| 10 | High-entropy `uaFullVersion`, or `NA` |
| 11 | JSON of high-entropy `fullVersionList`, or `NA` |
| 12 | `NA` only when `userAgentData` is absent |

Missing `getHighEntropyValues` sets slots 6 through 11 to `NA`. A thrown or
rejected high-entropy call sets them to `ERR`. Without `userAgentData`, slots
3 through 11 remain holes. The outer catch sends `[3, false, error]` instead.

The outer receiver handles the error tag before forwarding a successful
array. `Qon` at 506333..506739 selects its truthy element 10, otherwise its
existing string marker; the result enters `Q[242]` at 282619.
Thus standard API results supply a version string or a marker on this path.
This stock source does not establish an intact object producer for that slot.
The field assignment is not a runtime type check: altered API values remain
possible, and a configured replacement worker bypasses this stock literal.

This closes the stock source's schema question, not live worker selection,
operation completion, collector parity or service acceptance. The separate
`N_` / `krn` callback field feeding `Q[473]` still needs its producer trace.
Native implementation must bind actual collected inputs to these fields;
it must not invent browser observations from this schema.
