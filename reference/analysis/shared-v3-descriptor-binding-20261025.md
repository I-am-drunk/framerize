# Shared V3 service transition: descriptor binding

The first shared-service transition (R10) stopped before changing the service.
Its authenticated memory helper still compared the running binary and host
against the historical release. The intended rollback target was the repaired
R9 service, so executable validation correctly refused the mismatch.

The controller then reached its bounded descriptor-receive timeout. Collection
confirmed that R9 remained in the same generation with `Restart=always`.
No service switch, ingress restriction, or inference request occurred.
All 125 bound source inputs were unchanged; helpers, transient unit and socket
were absent, and the execution lock was free.

R11 preparation binds activation to the plan's rollback binary, host hash and
host path. The rollback direction continues to bind the candidate binary and
host. The original privileged validator remains byte-identical, including its
process generation, owner, parent, interpreter, script and descriptor checks.

Three isolated synthetic checks passed: activation binding, rollback binding,
and refusal of changed protocol source. Inputs remained unchanged and no case
was skipped. These checks qualify the adapter; they do not establish deployment
or native acceptance. The exact adapter and fixture are retained under
`scripts/tests/shared_descriptor_binding/` as source fixtures, not a launcher.

R10 collection: `5e9aa6bc25fd4fd1397c20c119a9bad81d4ec3ad24f9f30d6cb1caea2a92cfd1`.
Qualification: `11c01b182d0c694acbe2067bb376332074aea32bf989382687d76f5863d798ac`.
The fresh transition preserves the existing account pool and normal browser
routes. V3 remains an explicit endpoint with no native routing configuration.

R11 reached the corrected descriptor validation, then stopped before ingress
or memory transfer because the new plan omitted `interruption`. Its controller
exited at 05:31:22 UTC with the original R9 process still running. A separate
systemd stop/start at 05:33:15 changed the process generation while retaining
the repaired binary and automatic restart. The cause of that restart is not
established. R11 collection records it separately from the failed transition.

R12 preparation preserves the existing interruption policy. Both preparation
and pre-authentication validation now check its exact types, values and nft
binary hash. Three isolated checks passed: policy preservation by the real
builder, malformed-policy refusal, and controller refusal before runtime
validation. No cases were skipped. Their collection is
`bb69cae7b4c8fd4ad96084f2c72c691d9822e38a0a5290d2f8e08b9fa0a2628c`.
The exact tested functions and fixtures are under
`scripts/tests/shared_plan_policy/`. Full runtime qualification remains pending.
