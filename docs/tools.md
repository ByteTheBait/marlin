# Tools

Marlin calls these automatically — you don't invoke them manually:

| Tool               | What it does                                          |
|--------------------|-------------------------------------------------------|
| `read_file`        | Read a file (or just one function from a large file)  |
| `write_file`       | Create or overwrite a file                            |
| `edit_file`        | Targeted string replacement in a file                 |
| `notebook_edit`    | Replace, insert, or delete a cell in a Jupyter notebook |
| `run_command`      | Run a shell command                                   |
| `list_directory`   | List files and directories                            |
| `create_directory` | Create a directory                                    |
| `search_codebase`  | TF-IDF ranked search across the indexed project       |

Every file touched by an AI edit gets snapshotted first — use `/revert` to restore. See [Search & sessions](search-and-sessions.md).

You can also define your own LLM tools without touching Rust — see [Custom LLM tools](extending.md#custom-llm-tools--marlintools) in the extending guide.
