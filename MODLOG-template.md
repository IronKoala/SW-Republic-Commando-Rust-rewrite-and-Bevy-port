# MODLOG template

The MODLOG is a running record of every change to this project: what changed,
why, how it was checked, and what is still broken. Newest entries go at the top.
Add an entry after every change, including attempts that failed.

Based on the starter template at
https://ai-modding-guides-002f90.gitlab.io/starter-files/MODLOG-template/

## Entry format

Copy this block to the top of the log in `MODLOG.md` and fill it in.

```markdown
## YYYY-MM-DD: Short title of the change

- **Changed:** what changed, and in which files
- **Why:** the problem being solved or the goal
- **Tested how:** how it was checked (played the game, read a log, ran a test)
- **Result:** what happened, with numbers or log lines where possible
- **Still broken / not tested:** anything not verified, written plainly as "not tested"
- **Next:** the next step
```

## Rules of thumb

- If something was not run, write **"not tested"**. Never imply it was.
- Point to the log file, or paste the key lines, as evidence.
- Keep each field to a line or two.
- Log failed attempts too, so nobody repeats a dead end.
- The agent writes the entry itself as part of finishing a change.
