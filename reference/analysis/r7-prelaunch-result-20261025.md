# R7 prelaunch refusal and R8 preparation

R7 refused before controller output creation. Its coordination file had mode
0664; the existing private reader requires mode 0600 and one link. This was
a preparation defect. No service switch, request interruption, state capture
or acceptance request occurred. The original service generation was verified
unchanged, all 115 source hashes matched, both known launcher processes exited,
the transient unit disappeared, and the execution lock was released.

R7 failure collection:
`5baedf25d5db96283b25da688bbbff53fe15196eb00ef7d8e1ebfa52f7b16225`.
The consumed invocation and its original file metadata are retained.

R8 creates the coordination file through the existing private writer and
checks it with the same private reader used by the controller. Ownership,
mode 0600, one link and content digest passed. It reuses the unchanged,
qualified controller, stream checks, rollback code and staged candidate.
There is no new application build or qualification rerun.

R8 plan:
`0f840fefe253127187d2bb464d60618bc48ec8ac9132e31390096d0474064444`.
It executed after fresh local sudo authentication. Stream acceptance failed
and rollback completed. The collected outcome and its more informative
stream evidence are in `r8-stream-result-20261025.md`. This preparation note
does not establish successful deployment or interruption acceptance.
