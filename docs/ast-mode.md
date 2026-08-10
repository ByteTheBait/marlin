# AST Mode

AST mode changes how the LLM perceives and edits source files. Toggle it with `/ast`:

## `/ast off` (default)

Raw text mode. `read_file` returns the file's source as-is.

## `/ast sexpr`

Token-efficient exploration. `read_file` calls `ast-compiler decompile --format sexpr` instead of returning raw source, delivering a compact S-expression AST. Useful for navigating large files without burning through the context budget.

## `/ast harness`

Full structural surgery mode. Three additional LLM tools become active:

| Tool           | What it does                                                                 |
|----------------|------------------------------------------------------------------------------|
| `ast_skeleton` | Returns the API surface of a file (signatures only, no bodies) — start here  |
| `ast_get_node` | Returns full JSON for a single AST node by ID                                |
| `ast_mutate`   | Applies a structural edit to an AST node and auto-recompiles the source file |

`ast_mutate` supports three operations:

- **`str-replace`** — replace an AST node's JSON representation (`old_json` → `new_json`)
- **`append-stmt`** — append a statement JSON inside a node
- **`insert-before`** — insert a statement JSON before an index inside a node

After a mutation, Marlin automatically runs `ast-compiler compile` to regenerate the source file, then attempts an `optimize` pass. The LLM is instructed not to use `edit_file` while harness mode is active.

AST mode persists across sessions (stored in `~/.marlin/config.json`).

Requires the optional `ast-compiler` and `ast-harness` external tools — see [Install](../README.md#install).
