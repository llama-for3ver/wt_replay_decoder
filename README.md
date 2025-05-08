# WT Replay Decoder
[![crates.io](https://img.shields.io/crates/v/wrpl.svg)](https://crates.io/crates/wrpl)
[![Documentation](https://docs.rs/wrpl/badge.svg)](https://docs.rs/wrpl)
<!-- [![Build Status](https://img.shields.io/github/actions/workflow/status/llama-for3ver/wt_replay_decoder/ci.yml?branch=main&label=build&logo=github)](https://github.com/llama-for3ver/wt_replay_decoder/actions) -->
A Rust library/CLI to decode and parse War Thunder replays, sometimes successfully.

## Usage

### CLI
```shell
cargo run -- -r my_replay.wrpl
```
[todo]

## TODO/Roadmap
- [X] Parse headers (client & server)
- [x] Parse chat (client only)
- [X] CLI application to parse single replays
    - [ ] Parse entire folder/install (and provide stats?)
    - [ ] More intelligently detecting ZLIB offsets
    - [ ] Parse replays.wdb 
    - [ ] Possibly seperate, as lib end-users don't want CLI deps
- [X] Download an entire game (CLI)
- [ ] __Support server replays for basic packet parsing__
    - [ ] __Parse chat messages__
    - [ ] Link multiple [server] wrpls together for parsing
- [ ] __Get more information out of replays__
    - [ ] Vehicles, shells, positions, etc.
- [ ] Generally make more extensible/maintainable


