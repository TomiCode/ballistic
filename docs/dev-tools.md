# Developer tools

## Workspace layout

| Path | What |
|---|---|
| `crates/ckfile` | Reader for Virtools composition files (`.nmo` / `.cmo`, file version 8): container, object table, `StateChunk` sections and sub-chunks. Used by the asset packer. Never used by the game at runtime. |
| `tools/ckinspect` | CLI for looking inside those files while reverse-engineering |

## Building

```bash
cargo build --release -p ckinspect
```

## `ckinspect`

```bash
ckinspect info <file>                         # header, class histogram, plugin deps
ckinspect objects <file> [--class Mesh] [--sections]
ckinspect chunk <file> <index|name> [--max-words N]   # annotated word dump of one object
ckinspect json <file>                         # everything as JSON
ckinspect scan <dir>                          # parse every .nmo/.cmo, report failures
ckinspect identifiers <dir> [--class Texture] # section identifier statistics per class
```

`chunk` prints each identifier section as hex words with a best-guess reading
next to them (small ints, floats, ASCII).

## Tests against your own install

Unit tests use synthetic data only. To also run the tests against real game
files, point `BALLANCE_DIR` at the folder that contains `base.cmo`:

```bash
BALLANCE_DIR="/path/to/Ballance" cargo test --workspace
```

Without the variable, those tests skip themselves and pass.
