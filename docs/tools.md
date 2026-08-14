# Tools

Marlin calls these automatically — you don't invoke them manually:

| Tool               | What it does                                          |
|--------------------|-------------------------------------------------------|
| `read_file`        | Read a file (or just one function from a large file)  |
| `write_file`       | Create or overwrite a file                            |
| `edit_file`        | Targeted string replacement in a file                 |
| `notebook_edit`    | Replace, insert, or delete a cell in a Jupyter notebook |
| `run_command`      | Run a shell command (with optional timeout)           |
| `list_directory`   | List files and directories                            |
| `create_directory` | Create a directory                                    |
| `search_codebase`  | TF-IDF ranked search across the indexed project       |
| `search_symbols`   | Find which file defines a function/type/class/etc     |
| `grep`             | Regex search of file contents with line numbers/context |
| `glob`             | Find files by path pattern                            |
| `bg_start`         | Launch a long-running process in the background (id)  |
| `bg_status`        | Report status/exit code of a background process       |
| `bg_log`           | Read new output from a background process             |
| `bg_kill`          | Terminate a background process                        |

Every file touched by an AI edit gets snapshotted first — use `/revert` to restore. See [Search & sessions](search-and-sessions.md).

Long-running work (a dev server, a watch build, a long test) shouldn't block the conversation — start it with `bg_start`, keep working, and poll its output with `bg_status`/`bg_log`. The status bar shows a `⚙ N bg` chip while background processes are alive.

You can also define your own LLM tools without touching Rust — see [Custom LLM tools](extending.md#custom-llm-tools--marlintools) in the extending guide.
