# Codebase search & sessions

## Codebase search

Run `/index` once to build a TF-IDF index of your project. After that, Marlin automatically searches it before reading files, so it can navigate large codebases without reading every file.

```
/index            # build
/index status     # check stats
/search auth jwt  # manual search
```

The index is saved to `~/.marlin/index/` and updated automatically when Marlin writes or edits a file.

---

## Sessions & snapshots

Conversations are saved automatically at the end of each goal. Restore the last one with `/resume` or browse with `/history`.

File snapshots are taken before every AI edit. If something goes wrong:

```
/revert src/main.rs        # list snapshots
/revert src/main.rs 1      # restore the most recent one
```
