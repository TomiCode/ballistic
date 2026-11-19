# Developer tools

## Workspace layout

| Path | What |
|---|---|
| `crates/ckfile` | Reader for Virtools composition files (`.nmo` / `.cmo`, file version 8): container, object table, `StateChunk` sections and sub-chunks. Used by the asset packer. Never used by the game at runtime. |
| `crates/ckscene` | Decoders for scene classes on top of `ckfile`: `Mesh`, `Material`, `Texture`, `Entity3d` (3D objects, lights, cameras), `Group` |
| `crates/ballistic-bundle` | Bundle format: manifest and scene types, zip reader/writer. The engine's only data dependency |
| `crates/ballistic-pack` | The packer (library and CLI): install verification, scene/texture/audio conversion |
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
ckinspect scene-check <dir>                   # decode every scene object, report failures
ckinspect scene <file> [--class Material]     # decoded scene objects as JSON
ckinspect render <file> --out x.png [--textures <install>/Textures]
          [--azimuth 35 --elevation 35 --size 2048 --include-hidden]
```

`chunk` prints each identifier section as hex words with a best-guess reading
next to them (small ints, floats, ASCII).

`render` is a small software rasteriser for checking decoders by eye. It draws an
orthographic view of all visible geometry with textures. The PNG it writes
contains game imagery, so save it outside the repository.

## Tests against your own install

Unit tests use synthetic data only. To also run the tests against real game
files, point `BALLANCE_DIR` at the folder that contains `base.cmo`:

```bash
BALLANCE_DIR="/path/to/Ballance" cargo test --workspace
```

Without the variable, those tests skip themselves and pass.
