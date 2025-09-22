meta:
  id: replay_header
  file-extension: wrpl
  endian: le

seq:
  - id: magic
    type: u4
  - id: version
    type: u4
  - id: level
    type: str
    size: 128
    encoding: ASCII
  - id: level_settings
    type: str
    size: 260
    encoding: ASCII
  - id: battle_type
    type: str
    size: 128
    encoding: ASCII
  - id: environment
    type: str
    size: 128
    encoding: ASCII
  - id: visibility
    type: str
    size: 32
    encoding: ASCII
  - id: rez_offset
    type: u4
  - id: diff
    type: difficulty
  - id: padding1
    size: 35
  - id: session_type
    type: u4
  - id: padding2
    size: 4
  - id: session_id_hex
    type: u8
  - id: padding3
    size: 4
  - id: mset_size
    type: u4
  - id: padding4
    size: 32
  - id: loc_name
    type: str
    size: 128
    encoding: ASCII
  - id: start_time
    type: u4
  - id: time_limit
    type: u4
  - id: score_limit
    type: u4
  - id: padding5
    size: 48
  - id: battle_class
    type: str
    size: 128
    encoding: ASCII
  - id: battle_kill_streak
    type: str
    size: 128
    encoding: ASCII

types:
  difficulty:
    seq:
      - id: unk_nib
        type: b4
      - id: difficulty
        type: b4
