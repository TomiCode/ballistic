# Building your asset bundle (`ballistic-pack`)

Ballistic ships no game content. `ballistic-pack` reads **your own** Ballance
installation and converts it into `ballance.bpk`, the only file the game
loads. Nothing is uploaded or downloaded, and the bundle is for your personal use.

## Usage

```bash
ballistic-pack build --from "/path/to/Ballance" --out ballance.bpk
```

`--from` is the game folder (the one containing `base.cmo`). Pointing at its
`Bin` subfolder works too. Without `--from`, the packer tries `BALLANCE_DIR` and
a few common install locations (GOG on Windows, default Wine prefixes on
Linux/macOS). On Linux and macOS you can also copy the install folder from a
Windows machine. File-name case doesn't matter.

| Option | Default | Meaning |
|---|---|---|
| `--out <file>` | `ballance.bpk` | Output path. Written atomically: a failed run never leaves a half-written bundle |
| `--texture-encoding bc\|rgba8` | `bc` | GPU-compressed (BC1/BC3) or uncompressed textures |
| `--audio-encoding vorbis\|wav` | `vorbis` | Ogg Vorbis or 16-bit PCM WAV |
| `--audio-quality <q>` | `0.6` | Vorbis quality, -0.1 … 1.0 |
| `--jobs <n>` | all cores | Worker threads |
| `-v, --verbose` | | List every warning |

Other commands:

```bash
ballistic-pack verify --from "/path/to/Ballance"   # compare files with known releases
ballistic-pack inspect ballance.bpk               # summarise a bundle
```

## What it does

1. **Verifies** the install. Every file it reads is hashed (SHA-256) and compared
   with known releases (currently: GOG). Modified files, e.g. from mods, are
   reported as warnings and still packed.
2. **Converts** in parallel:
   - levels, prefabs, ball models and menu scenes → engine scenes (GPU-ready
     vertex/index buffers, materials, objects, groups), converted to a
     right-handed coordinate system. Files without geometry are skipped, except
     prefabs (`3D Entities/PH/`), whose frames the engine uses (e.g. a
     checkpoint's flame position)
   - textures (BMP/TGA) → KTX2 with a full mip chain, BC1 (opaque) or BC3 (alpha)
   - sounds (WAV) → Ogg Vorbis
   - physics parameters (gravity, time factor, ball types, level surfaces,
     props) read from the original's scripts and tables, and each moving
     element's bodies, joints, forces, timers and tests from its prefab script
     (`PH/P_Modul_*.nmo`, and the level end's `PH/PE_Balloon.nmo` without its
     UFO sequence), and the ball transformers' reach, pull and delays from
     `Gameplay.nmo`'s `Trafo Manager`, their animation from `AnimTrafo.nmo`,
     and each ball type's burst into pieces from `Balls.nmo` (progression
     curves decoded from their 2D curve parameters). Also their sounds: the
     Trafo Manager's messages to the sound scripts, followed by name into
     `Sound.nmo`, and the pieces' collision sounds. The ball's hit and roll
     sounds come from `Sound.nmo`'s `Sound_Manager` and its `BallSound` table,
     the level sound groups from `Levelinit.nmo`, and the prefab objects'
     hit and roll IDs from their attributes → `data/physics.json`
3. **Writes** the bundle (see [bundle-format.md](bundle-format.md)).

The same install with the same packer version and options always produces a
byte-identical bundle.

A full GOG install packs in about 2 seconds into roughly 19 MiB (vs. ~130 MiB of
source files).

## Not packed (yet)

- Low-detail variants (`Textures/sky_low`, `Sounds_low`): not needed on current hardware
- The intro video (`Textures/atari.avi`)
- Other game logic data from the original scripts (scores, timings, level flow)
- Gameplay meaning of level groups (spawn, checkpoints, sectors) is kept as group
  names for now, not yet resolved into structures
