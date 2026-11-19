# Ballistic

A from-scratch, cross-platform reimplementation of the *Ballance* engine,
written in Rust. It targets Windows, Linux and macOS, with any resolution and
windowed play.

> **Status: pre-alpha.** The asset packer and a level viewer work, and a physics
> ball can be rolled on the levels (`ballistic-viewer --play`). No game rules yet.

## You need your own copy of Ballance

Ballistic ships **no game content**. You build a local asset bundle from your own
legally obtained installation (e.g. the GOG release):

```bash
ballistic-pack build --from "/path/to/Ballance" --out ballance.bpk
```

The packer converts the original files into Ballistic's own optimised formats
(GPU-compressed textures, compressed audio, preprocessed level data). See
[docs/packer.md](docs/packer.md). Ballistic itself never reads the original game files. Then point the game
at the bundle. The bundle is for your personal use. Don't redistribute it.

## Documentation

See [`docs/`](docs/) for setup, features and development notes.

## License

MIT, see [LICENSE](LICENSE). *Ballance* is a trademark of its respective owners.
This project is not affiliated with or endorsed by them.
